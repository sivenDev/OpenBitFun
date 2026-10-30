//! Cross-account device connections.
//!
//! One device creates a one-time invite; a device of another account accepts
//! it; the inviting device confirms. Only then may the two devices route
//! delegation RPC to each other (`realtime::connection_call`). Either account
//! may revoke the connection at any time, which takes effect on the next
//! routing decision.
//!
//! Records hold identity facts only: accounts, devices, the device public keys
//! pinned when each side joined, and display names. Identity always comes from
//! the bearer credential, never from the request body. A connection is visible
//! to the two accounts it joins and to nobody else; an invite is visible to
//! whoever holds its code.
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use socketioxide::SocketIo;

use crate::db::{AuthToken, DbPool};
use crate::routes::api::AppState;

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS connection_invites (
  invite_id           TEXT PRIMARY KEY,
  code_hash           TEXT UNIQUE NOT NULL,
  inviter_user_id     TEXT NOT NULL,
  inviter_device_id   TEXT NOT NULL,
  inviter_public_key  TEXT NOT NULL,
  inviter_display     TEXT NOT NULL,
  inviter_device_name TEXT NOT NULL,
  state               TEXT NOT NULL DEFAULT 'open',
  connection_id       TEXT,
  created_at          INTEGER NOT NULL,
  expires_at          INTEGER NOT NULL,
  FOREIGN KEY (inviter_user_id, inviter_device_id)
    REFERENCES devices(user_id, device_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_connection_invites_inviter
  ON connection_invites(inviter_user_id);
CREATE TABLE IF NOT EXISTS device_connections (
  connection_id        TEXT PRIMARY KEY,
  inviter_user_id      TEXT NOT NULL,
  inviter_device_id    TEXT NOT NULL,
  inviter_public_key   TEXT NOT NULL,
  inviter_display      TEXT NOT NULL,
  inviter_device_name  TEXT NOT NULL,
  acceptor_user_id     TEXT NOT NULL,
  acceptor_device_id   TEXT NOT NULL,
  acceptor_public_key  TEXT NOT NULL,
  acceptor_display     TEXT NOT NULL,
  acceptor_device_name TEXT NOT NULL,
  status               TEXT NOT NULL,
  created_at           INTEGER NOT NULL,
  confirmed_at         INTEGER,
  revoked_at           INTEGER,
  revoked_by_user_id   TEXT,
  revoked_reason       TEXT
);
CREATE INDEX IF NOT EXISTS idx_device_connections_inviter
  ON device_connections(inviter_user_id);
CREATE INDEX IF NOT EXISTS idx_device_connections_acceptor
  ON device_connections(acceptor_user_id);
CREATE TRIGGER IF NOT EXISTS revoke_connections_of_removed_device
AFTER DELETE ON devices
BEGIN
  UPDATE device_connections
     SET status = 'revoked', revoked_at = unixepoch(), revoked_reason = 'device_removed'
   WHERE status != 'revoked'
     AND ((inviter_user_id = OLD.user_id AND inviter_device_id = OLD.device_id)
       OR (acceptor_user_id = OLD.user_id AND acceptor_device_id = OLD.device_id));
END;
"#;

const DEFAULT_INVITE_TTL_SECS: i64 = 60 * 60;
const MIN_INVITE_TTL_SECS: i64 = 60;
const MAX_INVITE_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// Invites past their expiry stay readable for a day so a late acceptor learns
/// that the invite expired rather than that it never existed.
const EXPIRED_INVITE_RETENTION_SECS: i64 = 24 * 60 * 60;
const MAX_OPEN_INVITES_PER_ACCOUNT: i64 = 16;
const MAX_CONNECTIONS_PER_ACCOUNT: i64 = 256;
const MAX_LISTED: i64 = 512;
const MAX_CODE_BYTES: usize = 128;
const MAX_ID_BYTES: usize = 128;

/// Device dispatch commands a connection may carry. The payload stays
/// encrypted end to end; only the command name is visible to the relay, and
/// the target must check that the decrypted request names the same command.
const ROUTABLE_COMMANDS: &[&str] = &[
    "dispatch_target_probe",
    "dispatch_target_catalog",
    "dispatch_target_submit",
    "dispatch_target_status",
    "dispatch_target_query",
    "dispatch_target_list",
    "dispatch_target_cancel",
    "dispatch_target_answer",
    "dispatch_target_append",
    "dispatch_target_continue",
    "dispatch_target_workspace_provision",
    "dispatch_target_workspace_sync",
    "dispatch_target_workspace_sync_chunk",
    "dispatch_target_workspace_bundle_begin",
    "dispatch_target_workspace_bundle_chunk",
    "dispatch_target_workspace_bundle_commit",
];

pub(crate) fn is_routable_command(command: &str) -> bool {
    ROUTABLE_COMMANDS.contains(&command)
}

pub(crate) fn is_valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub(crate) async fn ensure_schema(pool: &DbPool) -> anyhow::Result<()> {
    sqlx::raw_sql(SCHEMA)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("create connection tables: {e}"))?;
    sqlx::query("DELETE FROM connection_invites WHERE expires_at <= unixepoch() - ?")
        .bind(EXPIRED_INVITE_RETENTION_SECS)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("clean expired connection invites: {e}"))?;
    Ok(())
}

// ── Routing decision ───────────────────────────────────────────────────

/// Why the relay will not route over a connection. Each has its own reason
/// code so a client can tell the user what to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// No such connection, or the caller's device is not one of its ends.
    NotFound,
    Pending,
    Revoked,
    /// A device registered a different public key after pairing; the
    /// connection must be paired again.
    KeyChanged,
}

impl Refusal {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::NotFound => "connection_not_found",
            Self::Pending => "connection_pending",
            Self::Revoked => "connection_revoked",
            Self::KeyChanged => "connection_key_changed",
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::NotFound => "cross-account connection not found",
            Self::Pending => "cross-account connection is waiting for confirmation",
            Self::Revoked => "cross-account connection was revoked",
            Self::KeyChanged => "a device key changed since pairing; pair the devices again",
        }
    }
}

pub(crate) struct Route {
    pub(crate) target_user_id: String,
    pub(crate) target_device_id: String,
}

/// Decide whether `caller` may reach the other end of `connection_id` now.
pub(crate) async fn route(
    pool: &DbPool,
    connection_id: &str,
    caller_user_id: &str,
    caller_device_id: &str,
) -> Result<Result<Route, Refusal>, sqlx::Error> {
    let Some(row) = ConnectionRow::find(pool, connection_id).await? else {
        return Ok(Err(Refusal::NotFound));
    };
    let Some(role) = row.role_of_device(caller_user_id, caller_device_id) else {
        return Ok(Err(Refusal::NotFound));
    };
    match row.status.as_str() {
        "active" => {}
        "pending" => return Ok(Err(Refusal::Pending)),
        _ => return Ok(Err(Refusal::Revoked)),
    }
    let keys_match = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT (SELECT public_key FROM devices WHERE user_id = ?1 AND device_id = ?2) = ?3
            AND (SELECT public_key FROM devices WHERE user_id = ?4 AND device_id = ?5) = ?6",
    )
    .bind(&row.inviter_user_id)
    .bind(&row.inviter_device_id)
    .bind(&row.inviter_public_key)
    .bind(&row.acceptor_user_id)
    .bind(&row.acceptor_device_id)
    .bind(&row.acceptor_public_key)
    .fetch_one(pool)
    .await?;
    if keys_match != Some(1) {
        return Ok(Err(Refusal::KeyChanged));
    }
    let (target_user_id, target_device_id) = match role {
        Role::Inviter => (row.acceptor_user_id, row.acceptor_device_id),
        Role::Acceptor => (row.inviter_user_id, row.inviter_device_id),
    };
    Ok(Ok(Route {
        target_user_id,
        target_device_id,
    }))
}

// ── Records ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Inviter,
    Acceptor,
}

#[derive(sqlx::FromRow)]
struct ConnectionRow {
    connection_id: String,
    inviter_user_id: String,
    inviter_device_id: String,
    inviter_public_key: String,
    inviter_display: String,
    inviter_device_name: String,
    acceptor_user_id: String,
    acceptor_device_id: String,
    acceptor_public_key: String,
    acceptor_display: String,
    acceptor_device_name: String,
    status: String,
    created_at: i64,
    confirmed_at: Option<i64>,
    revoked_at: Option<i64>,
    revoked_reason: Option<String>,
}

impl ConnectionRow {
    async fn find<'e, E>(executor: E, connection_id: &str) -> Result<Option<Self>, sqlx::Error>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        sqlx::query_as::<_, Self>("SELECT * FROM device_connections WHERE connection_id = ?")
            .bind(connection_id)
            .fetch_optional(executor)
            .await
    }

    fn role_of_account(&self, user_id: &str) -> Option<Role> {
        if self.inviter_user_id == user_id {
            Some(Role::Inviter)
        } else if self.acceptor_user_id == user_id {
            Some(Role::Acceptor)
        } else {
            None
        }
    }

    fn role_of_device(&self, user_id: &str, device_id: &str) -> Option<Role> {
        match self.role_of_account(user_id)? {
            Role::Inviter if self.inviter_device_id == device_id => Some(Role::Inviter),
            Role::Acceptor if self.acceptor_device_id == device_id => Some(Role::Acceptor),
            _ => None,
        }
    }

    fn view_for(&self, role: Role) -> ConnectionView {
        let (local_device_id, peer) = match role {
            Role::Inviter => (
                &self.inviter_device_id,
                PeerView {
                    user_id: self.acceptor_user_id.clone(),
                    display_name: self.acceptor_display.clone(),
                    device_id: self.acceptor_device_id.clone(),
                    device_name: self.acceptor_device_name.clone(),
                    public_key: self.acceptor_public_key.clone(),
                },
            ),
            Role::Acceptor => (
                &self.acceptor_device_id,
                PeerView {
                    user_id: self.inviter_user_id.clone(),
                    display_name: self.inviter_display.clone(),
                    device_id: self.inviter_device_id.clone(),
                    device_name: self.inviter_device_name.clone(),
                    public_key: self.inviter_public_key.clone(),
                },
            ),
        };
        ConnectionView {
            connection_id: self.connection_id.clone(),
            status: self.status.clone(),
            role: match role {
                Role::Inviter => "inviter",
                Role::Acceptor => "acceptor",
            },
            local_device_id: local_device_id.clone(),
            peer,
            created_at: self.created_at,
            confirmed_at: self.confirmed_at,
            revoked_at: self.revoked_at,
            revoked_reason: self.revoked_reason.clone(),
        }
    }
}

#[derive(Serialize)]
struct PeerView {
    user_id: String,
    display_name: String,
    device_id: String,
    device_name: String,
    public_key: String,
}

#[derive(Serialize)]
struct ConnectionView {
    connection_id: String,
    status: String,
    role: &'static str,
    local_device_id: String,
    peer: PeerView,
    created_at: i64,
    confirmed_at: Option<i64>,
    revoked_at: Option<i64>,
    revoked_reason: Option<String>,
}

#[derive(sqlx::FromRow)]
struct InviteRow {
    invite_id: String,
    inviter_user_id: String,
    inviter_device_id: String,
    inviter_public_key: String,
    inviter_display: String,
    inviter_device_name: String,
    state: String,
    created_at: i64,
    expires_at: i64,
}

impl InviteRow {
    /// Why an invite can no longer be accepted, if it cannot.
    fn closed_reason(&self, now: i64) -> Option<&'static str> {
        match self.state.as_str() {
            "used" => Some("invite_used"),
            "withdrawn" => Some("invite_withdrawn"),
            _ if self.expires_at <= now => Some("invite_expired"),
            _ => None,
        }
    }
}

/// The caller's own device, read inside the write transaction so a revoked
/// credential or a removed device cannot create or confirm trust.
struct LocalDevice {
    public_key: String,
    display: String,
    device_name: String,
}

// ── HTTP surface ───────────────────────────────────────────────────────

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/connections", get(list_connections))
        .route(
            "/api/connections/invites",
            post(create_invite).get(list_invites),
        )
        .route("/api/connections/invites/preview", post(preview_invite))
        .route(
            "/api/connections/invites/{invite_id}",
            delete(withdraw_invite),
        )
        .route("/api/connections/accept", post(accept_invite))
        .route(
            "/api/connections/{connection_id}/confirm",
            post(confirm_connection),
        )
        .route(
            "/api/connections/{connection_id}",
            delete(revoke_connection),
        )
}

struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "reason": self.1 }))).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(%error, "Connection store query failed");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable")
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn code_hash(code: &str) -> String {
    format!("{:x}", Sha256::digest(code.as_bytes()))
}

fn is_valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_CODE_BYTES
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Connections bind devices, so only a device credential may manage them;
/// delegated controller credentials are refused.
async fn device_caller(state: &AppState, headers: &HeaderMap) -> Result<AuthToken, ApiError> {
    let auth = crate::routes::devices::validate_user(state, headers)
        .await
        .map_err(|status| {
            ApiError(
                status,
                if status == StatusCode::FORBIDDEN {
                    "device_credential_required"
                } else {
                    "unauthorized"
                },
            )
        })?;
    if !auth.is_device_token() {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "device_credential_required",
        ));
    }
    Ok(auth)
}

async fn local_device(
    tx: &mut sqlx::SqliteConnection,
    auth: &AuthToken,
) -> Result<LocalDevice, ApiError> {
    let current = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM auth_tokens WHERE token = ? AND user_id = ? AND device_id = ?
           AND token_kind = 'device' AND expires_at > unixepoch()",
    )
    .bind(&auth.token)
    .bind(&auth.user_id)
    .bind(&auth.device_id)
    .fetch_one(&mut *tx)
    .await?;
    if current != 1 {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "unauthorized"));
    }
    let row = sqlx::query_as::<_, (Option<String>, String, String)>(
        "SELECT d.public_key, COALESCE(d.device_alias, d.device_name, d.device_id), u.username
           FROM devices d JOIN users u ON u.user_id = d.user_id
          WHERE d.user_id = ? AND d.device_id = ?",
    )
    .bind(&auth.user_id)
    .bind(&auth.device_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    let public_key = row
        .0
        .filter(|key| !key.trim().is_empty())
        .ok_or(ApiError(StatusCode::CONFLICT, "device_public_key_missing"))?;
    Ok(LocalDevice {
        public_key,
        device_name: row.1,
        display: row.2,
    })
}

/// Tell both ends' devices that a connection changed, so a waiting inviter
/// can confirm and a revoked peer stops at once. Delivery is best effort;
/// devices also re-read the list when they reconnect.
fn notify(io: &SocketIo, row: &ConnectionRow, status: &str) {
    let event = json!({
        "type": "connection-updated",
        "connectionId": row.connection_id,
        "status": status,
    });
    for (user, device) in [
        (&row.inviter_user_id, &row.inviter_device_id),
        (&row.acceptor_user_id, &row.acceptor_device_id),
    ] {
        for socket in io.within(format!("user:{user}:device:{device}")).sockets() {
            let _ = socket.emit("ephemeral", &event);
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CreateInvite {
    #[serde(default)]
    expires_in_seconds: Option<i64>,
}

#[derive(Serialize)]
struct CreatedInvite {
    invite_id: String,
    code: String,
    expires_at: i64,
}

async fn create_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<CreateInvite>>,
) -> Result<(StatusCode, Json<CreatedInvite>), ApiError> {
    let auth = device_caller(&state, &headers).await?;
    let ttl = body
        .and_then(|Json(body)| body.expires_in_seconds)
        .unwrap_or(DEFAULT_INVITE_TTL_SECS);
    if !(MIN_INVITE_TTL_SECS..=MAX_INVITE_TTL_SECS).contains(&ttl) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid_expiry"));
    }
    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    let device = local_device(&mut tx, &auth).await?;
    let now = now();
    let open = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM connection_invites
          WHERE inviter_user_id = ? AND state = 'open' AND expires_at > ?",
    )
    .bind(&auth.user_id)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    if open >= MAX_OPEN_INVITES_PER_ACCOUNT {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "invite_quota_exceeded",
        ));
    }
    let code = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let invite = CreatedInvite {
        invite_id: uuid::Uuid::new_v4().to_string(),
        code,
        expires_at: now + ttl,
    };
    sqlx::query(
        "INSERT INTO connection_invites
           (invite_id, code_hash, inviter_user_id, inviter_device_id, inviter_public_key,
            inviter_display, inviter_device_name, state, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'open', ?, ?)",
    )
    .bind(&invite.invite_id)
    .bind(code_hash(&invite.code))
    .bind(&auth.user_id)
    .bind(&auth.device_id)
    .bind(&device.public_key)
    .bind(&device.display)
    .bind(&device.device_name)
    .bind(now)
    .bind(invite.expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(invite)))
}

#[derive(Serialize)]
struct InviteListEntry {
    invite_id: String,
    device_id: String,
    created_at: i64,
    expires_at: i64,
}

async fn list_invites(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<InviteListEntry>>, ApiError> {
    let auth = device_caller(&state, &headers).await?;
    let rows = sqlx::query_as::<_, InviteRow>(
        "SELECT * FROM connection_invites
          WHERE inviter_user_id = ? AND state = 'open' AND expires_at > ?
          ORDER BY created_at DESC LIMIT ?",
    )
    .bind(&auth.user_id)
    .bind(now())
    .bind(MAX_LISTED)
    .fetch_all(state.db.as_ref())
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| InviteListEntry {
                invite_id: row.invite_id,
                device_id: row.inviter_device_id,
                created_at: row.created_at,
                expires_at: row.expires_at,
            })
            .collect(),
    ))
}

async fn withdraw_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(invite_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let auth = device_caller(&state, &headers).await?;
    if !is_valid_id(&invite_id) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid_invite_id"));
    }
    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    let invite = sqlx::query_as::<_, InviteRow>(
        "SELECT * FROM connection_invites WHERE invite_id = ? AND inviter_user_id = ?",
    )
    .bind(&invite_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError(StatusCode::NOT_FOUND, "invite_not_found"))?;
    match invite.state.as_str() {
        "used" => return Err(ApiError(StatusCode::CONFLICT, "invite_used")),
        "withdrawn" => return Ok(StatusCode::NO_CONTENT),
        _ => {}
    }
    sqlx::query("UPDATE connection_invites SET state = 'withdrawn' WHERE invite_id = ?")
        .bind(&invite_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteCode {
    code: String,
}

#[derive(Serialize)]
struct InvitePreview {
    inviter: PeerView,
    expires_at: i64,
    same_account: bool,
}

async fn open_invite<'e, E>(executor: E, code: &str) -> Result<InviteRow, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    if !is_valid_code(code) {
        return Err(ApiError(StatusCode::NOT_FOUND, "invite_not_found"));
    }
    let invite =
        sqlx::query_as::<_, InviteRow>("SELECT * FROM connection_invites WHERE code_hash = ?")
            .bind(code_hash(code))
            .fetch_optional(executor)
            .await?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "invite_not_found"))?;
    if let Some(reason) = invite.closed_reason(now()) {
        return Err(ApiError(StatusCode::GONE, reason));
    }
    Ok(invite)
}

/// Shows who sent an invite so the receiver can check the fingerprint before
/// accepting. Holding the code is what grants this view.
async fn preview_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<InviteCode>,
) -> Result<Json<InvitePreview>, ApiError> {
    let auth = device_caller(&state, &headers).await?;
    let invite = open_invite(state.db.as_ref(), &body.code).await?;
    Ok(Json(InvitePreview {
        same_account: invite.inviter_user_id == auth.user_id,
        expires_at: invite.expires_at,
        inviter: PeerView {
            user_id: invite.inviter_user_id,
            display_name: invite.inviter_display,
            device_id: invite.inviter_device_id,
            device_name: invite.inviter_device_name,
            public_key: invite.inviter_public_key,
        },
    }))
}

async fn non_revoked_connections(
    tx: &mut sqlx::SqliteConnection,
    user_id: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM device_connections
          WHERE status != 'revoked' AND (inviter_user_id = ?1 OR acceptor_user_id = ?1)",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
}

async fn accept_invite(
    State(state): State<AppState>,
    Extension(io): Extension<SocketIo>,
    headers: HeaderMap,
    Json(body): Json<InviteCode>,
) -> Result<(StatusCode, Json<ConnectionView>), ApiError> {
    let auth = device_caller(&state, &headers).await?;
    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    let invite = open_invite(&mut *tx, &body.code).await?;
    if invite.inviter_user_id == auth.user_id {
        // Devices of one account already reach each other directly.
        return Err(ApiError(StatusCode::BAD_REQUEST, "same_account"));
    }
    let device = local_device(&mut tx, &auth).await?;
    for user in [&invite.inviter_user_id, &auth.user_id] {
        if non_revoked_connections(&mut tx, user).await? >= MAX_CONNECTIONS_PER_ACCOUNT {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "connection_quota_exceeded",
            ));
        }
    }
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM device_connections
          WHERE status != 'revoked'
            AND ((inviter_user_id = ?1 AND inviter_device_id = ?2
                  AND acceptor_user_id = ?3 AND acceptor_device_id = ?4)
              OR (inviter_user_id = ?3 AND inviter_device_id = ?4
                  AND acceptor_user_id = ?1 AND acceptor_device_id = ?2))",
    )
    .bind(&invite.inviter_user_id)
    .bind(&invite.inviter_device_id)
    .bind(&auth.user_id)
    .bind(&auth.device_id)
    .fetch_one(&mut *tx)
    .await?;
    if existing > 0 {
        return Err(ApiError(StatusCode::CONFLICT, "connection_exists"));
    }
    let row = ConnectionRow {
        connection_id: uuid::Uuid::new_v4().to_string(),
        inviter_user_id: invite.inviter_user_id,
        inviter_device_id: invite.inviter_device_id,
        inviter_public_key: invite.inviter_public_key,
        inviter_display: invite.inviter_display,
        inviter_device_name: invite.inviter_device_name,
        acceptor_user_id: auth.user_id.clone(),
        acceptor_device_id: auth.device_id.clone(),
        acceptor_public_key: device.public_key,
        acceptor_display: device.display,
        acceptor_device_name: device.device_name,
        status: "pending".to_string(),
        created_at: now(),
        confirmed_at: None,
        revoked_at: None,
        revoked_reason: None,
    };
    sqlx::query(
        "INSERT INTO device_connections
           (connection_id, inviter_user_id, inviter_device_id, inviter_public_key,
            inviter_display, inviter_device_name, acceptor_user_id, acceptor_device_id,
            acceptor_public_key, acceptor_display, acceptor_device_name, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?)",
    )
    .bind(&row.connection_id)
    .bind(&row.inviter_user_id)
    .bind(&row.inviter_device_id)
    .bind(&row.inviter_public_key)
    .bind(&row.inviter_display)
    .bind(&row.inviter_device_name)
    .bind(&row.acceptor_user_id)
    .bind(&row.acceptor_device_id)
    .bind(&row.acceptor_public_key)
    .bind(&row.acceptor_display)
    .bind(&row.acceptor_device_name)
    .bind(row.created_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE connection_invites SET state = 'used', connection_id = ? WHERE invite_id = ?",
    )
    .bind(&row.connection_id)
    .bind(&invite.invite_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    notify(&io, &row, "pending");
    Ok((StatusCode::CREATED, Json(row.view_for(Role::Acceptor))))
}

async fn connection_for_account(
    tx: &mut sqlx::SqliteConnection,
    connection_id: &str,
    user_id: &str,
) -> Result<(ConnectionRow, Role), ApiError> {
    if !is_valid_id(connection_id) {
        return Err(ApiError(StatusCode::NOT_FOUND, "connection_not_found"));
    }
    let row = ConnectionRow::find(&mut *tx, connection_id)
        .await?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "connection_not_found"))?;
    // Accounts outside the connection learn nothing, not even that it exists.
    let role = row
        .role_of_account(user_id)
        .ok_or(ApiError(StatusCode::NOT_FOUND, "connection_not_found"))?;
    Ok((row, role))
}

async fn confirm_connection(
    State(state): State<AppState>,
    Extension(io): Extension<SocketIo>,
    headers: HeaderMap,
    Path(connection_id): Path<String>,
) -> Result<Json<ConnectionView>, ApiError> {
    let auth = device_caller(&state, &headers).await?;
    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    local_device(&mut tx, &auth).await?;
    let (mut row, role) = connection_for_account(&mut tx, &connection_id, &auth.user_id).await?;
    // Only the device that issued the invite can complete the pairing.
    if role != Role::Inviter || row.inviter_device_id != auth.device_id {
        return Err(ApiError(StatusCode::FORBIDDEN, "inviter_device_required"));
    }
    match row.status.as_str() {
        "active" => return Ok(Json(row.view_for(role))),
        "pending" => {}
        _ => return Err(ApiError(StatusCode::CONFLICT, "connection_revoked")),
    }
    let confirmed_at = now();
    sqlx::query(
        "UPDATE device_connections SET status = 'active', confirmed_at = ?
          WHERE connection_id = ? AND status = 'pending'",
    )
    .bind(confirmed_at)
    .bind(&row.connection_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    row.status = "active".to_string();
    row.confirmed_at = Some(confirmed_at);
    notify(&io, &row, "active");
    Ok(Json(row.view_for(role)))
}

/// Any device of either account may revoke, so a lost device can be cut off
/// from another one.
async fn revoke_connection(
    State(state): State<AppState>,
    Extension(io): Extension<SocketIo>,
    headers: HeaderMap,
    Path(connection_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let auth = device_caller(&state, &headers).await?;
    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    local_device(&mut tx, &auth).await?;
    let (row, _) = connection_for_account(&mut tx, &connection_id, &auth.user_id).await?;
    if row.status == "revoked" {
        return Ok(StatusCode::NO_CONTENT);
    }
    sqlx::query(
        "UPDATE device_connections
            SET status = 'revoked', revoked_at = ?, revoked_by_user_id = ?, revoked_reason = 'revoked'
          WHERE connection_id = ?",
    )
    .bind(now())
    .bind(&auth.user_id)
    .bind(&row.connection_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    notify(&io, &row, "revoked");
    tracing::info!(connection_id = %row.connection_id, "Cross-account connection revoked");
    Ok(StatusCode::NO_CONTENT)
}

async fn list_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ConnectionView>>, ApiError> {
    let auth = device_caller(&state, &headers).await?;
    let rows = sqlx::query_as::<_, ConnectionRow>(
        "SELECT * FROM device_connections
          WHERE inviter_user_id = ?1 OR acceptor_user_id = ?1
          ORDER BY created_at DESC LIMIT ?2",
    )
    .bind(&auth.user_id)
    .bind(MAX_LISTED)
    .fetch_all(state.db.as_ref())
    .await?;
    Ok(Json(
        rows.iter()
            .filter_map(|row| row.role_of_account(&auth.user_id).map(|r| row.view_for(r)))
            .collect(),
    ))
}

#[cfg(test)]
mod tests;
