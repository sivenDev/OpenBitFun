//! Cross-account device connections: pairing, fingerprints, connection keys
//! and the local connection store.
//!
//! Two devices of different accounts pair through a one-time invite held by
//! the Relay (`cross_account_routing_v1`). The invite link carries the
//! inviter's key fingerprint, so a Relay that substitutes another key fails
//! the check on the accepting device. Each side pins the other device's key
//! when it joins; a later key change disables the connection instead of
//! accepting the new key. Requests and answers are encrypted with a key
//! derived from the two device keys and bound to the connection id; account
//! keys of either side never take part.
//!
//! The local store keeps every record it cannot parse, reports it as
//! disabled, and never deletes it.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use super::account::{validate_relay_base_url, AccountSession};
use super::device_crypto;
use super::relay_http::relay_http_client;

/// Relay capability that enables everything in this module.
pub const RELAY_CAPABILITY_CROSS_ACCOUNT_ROUTING: &str = "cross_account_routing_v1";

/// Method a device registers, as `<device>:connection-invoke`, to take calls
/// from paired devices of other accounts.
pub const CONNECTION_INVOKE_METHOD: &str = "connection-invoke";

/// Commands a connection may carry; the Relay holds the same list. A device
/// checks it again on arrival rather than trusting the Relay.
pub const CONNECTION_COMMANDS: &[&str] = &[
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

pub fn is_connection_command(command: &str) -> bool {
    CONNECTION_COMMANDS.contains(&command)
}

const CONNECTION_KDF_SALT: &[u8] = b"OpenBitFun cross-account connection v1";
const INVITE_LINK_PREFIX: &str = "openbitfun://connect?";
const STORE_VERSION: u32 = 1;

// ── Identity and keys ──────────────────────────────────────────────────

/// Human-checkable fingerprint of a device public key: the first 16 bytes of
/// its SHA-256, in eight groups of four hex digits.
pub fn fingerprint(public_key: &[u8; 32]) -> String {
    let digest = Sha256::digest(public_key);
    digest[..16]
        .chunks(2)
        .map(|pair| format!("{:02X}{:02X}", pair[0], pair[1]))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn fingerprint_of(public_key_base64: &str) -> Result<String> {
    Ok(fingerprint(&super::encryption::parse_public_key(
        public_key_base64,
    )?))
}

/// Fingerprints compare without regard to spacing or case.
pub fn same_fingerprint(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        value
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .flat_map(char::to_uppercase)
            .collect::<String>()
    };
    let left = normalize(left);
    !left.is_empty() && left == normalize(right)
}

/// X25519 between the two device keys, then HKDF bound to both keys and the
/// connection id, so each connection has its own key.
pub fn derive_connection_key(
    device_secret: &[u8; 32],
    peer_public: &[u8; 32],
    connection_id: &str,
) -> Result<[u8; 32]> {
    let local_public = device_crypto::public_key(device_secret);
    let shared = StaticSecret::from(*device_secret).diffie_hellman(&PublicKey::from(*peer_public));
    if !shared.was_contributory() {
        bail!("invalid peer public key");
    }
    let (first, second) = if local_public < *peer_public {
        (&local_public, peer_public)
    } else {
        (peer_public, &local_public)
    };
    let mut info = Vec::with_capacity(64 + connection_id.len());
    info.extend_from_slice(first);
    info.extend_from_slice(second);
    info.extend_from_slice(connection_id.as_bytes());
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(Some(CONNECTION_KDF_SALT), shared.as_bytes())
        .expand(&info, &mut key)
        .map_err(|_| anyhow!("connection key derivation failed"))?;
    Ok(key)
}

// ── Invite links ───────────────────────────────────────────────────────

/// What one side sends the other, by any channel the two people trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    pub relay_url: String,
    pub code: String,
    /// Fingerprint of the inviting device's key, without spaces.
    pub fingerprint: String,
}

impl std::fmt::Display for InviteLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{INVITE_LINK_PREFIX}relay={}&code={}&fp={}",
            urlencoding::encode(&self.relay_url),
            urlencoding::encode(&self.code),
            urlencoding::encode(&self.fingerprint.replace(' ', "")),
        )
    }
}

impl InviteLink {
    pub fn parse(value: &str) -> Result<Self> {
        let query = value
            .trim()
            .strip_prefix(INVITE_LINK_PREFIX)
            .ok_or_else(|| anyhow!("not an OpenBitFun connection invite"))?;
        let (mut relay_url, mut code, mut fingerprint) = (None, None, None);
        for pair in query.split('&') {
            let (key, raw) = pair.split_once('=').unwrap_or((pair, ""));
            let decoded = urlencoding::decode(raw)
                .map_err(|_| anyhow!("connection invite is malformed"))?
                .into_owned();
            match key {
                "relay" => relay_url = Some(decoded),
                "code" => code = Some(decoded),
                "fp" => fingerprint = Some(decoded),
                _ => {}
            }
        }
        let link = Self {
            relay_url: relay_url.ok_or_else(|| anyhow!("connection invite has no relay"))?,
            code: code
                .filter(|code| !code.is_empty())
                .ok_or_else(|| anyhow!("connection invite has no code"))?,
            fingerprint: fingerprint
                .filter(|fp| !fp.is_empty())
                .ok_or_else(|| anyhow!("connection invite has no fingerprint"))?,
        };
        validate_relay_base_url(&link.relay_url)?;
        Ok(link)
    }
}

fn same_relay(left: &str, right: &str) -> bool {
    left.trim_end_matches('/') == right.trim_end_matches('/')
}

// ── Relay API ──────────────────────────────────────────────────────────

/// A refusal from the Relay's connection API, with its stable reason code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRefusal {
    pub status: u16,
    pub reason: String,
}

impl std::fmt::Display for ConnectionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (HTTP {})", self.reason, self.status)
    }
}

impl std::error::Error for ConnectionRefusal {}

/// A connection call the Relay or the other device refused, with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRefusal {
    pub reason: String,
    pub message: String,
}

impl std::fmt::Display for RouteRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.reason)
    }
}

impl std::error::Error for RouteRefusal {}

/// The stable reason code of a connection failure, when it has one.
pub fn refusal_reason(error: &anyhow::Error) -> Option<&str> {
    if let Some(refusal) = error.downcast_ref::<ConnectionRefusal>() {
        return Some(&refusal.reason);
    }
    if let Some(refusal) = error.downcast_ref::<RouteRefusal>() {
        return Some(&refusal.reason);
    }
    if let Some(unusable) = error.downcast_ref::<ConnectionUnusable>() {
        return Some(&unusable.reason);
    }
    error
        .downcast_ref::<PairingError>()
        .map(PairingError::reason)
}

/// Pairing checks done on this device rather than by the Relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingError {
    /// The relay is not the one the invite was made on.
    DifferentRelay,
    /// The key the Relay reports does not match the invite's fingerprint.
    FingerprintMismatch,
    /// The relay cannot route between accounts.
    RelayUnsupported,
}

impl PairingError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::DifferentRelay => "different_relay",
            Self::FingerprintMismatch => "fingerprint_mismatch",
            Self::RelayUnsupported => "relay_unsupported",
        }
    }
}

impl std::fmt::Display for PairingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DifferentRelay => "the invite was made on another relay",
            Self::FingerprintMismatch => {
                "the device key reported by the relay does not match the invite fingerprint"
            }
            Self::RelayUnsupported => "the relay does not support cross-account connections",
        })
    }
}

impl std::error::Error for PairingError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayPeer {
    pub user_id: String,
    pub display_name: String,
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelayConnection {
    pub connection_id: String,
    pub status: String,
    pub role: ConnectionRole,
    pub local_device_id: String,
    pub peer: RelayPeer,
    pub created_at: i64,
    #[serde(default)]
    pub confirmed_at: Option<i64>,
    #[serde(default)]
    pub revoked_at: Option<i64>,
    #[serde(default)]
    pub revoked_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelayInvitePreview {
    pub inviter: RelayPeer,
    pub expires_at: i64,
    pub same_account: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CreatedInvite {
    invite_id: String,
    code: String,
    expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct OpenInvite {
    pub invite_id: String,
    pub device_id: String,
    pub created_at: i64,
    pub expires_at: i64,
}

struct RelayApi<'a> {
    relay_url: &'a str,
    session: &'a AccountSession,
}

impl RelayApi<'_> {
    fn url(&self, path: &str) -> Result<reqwest::Url> {
        let mut url = validate_relay_base_url(self.relay_url)?;
        let base = url.path().trim_end_matches('/').to_string();
        url.set_path(&format!("{base}{path}"));
        Ok(url)
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = relay_http_client()
            .request(method, self.url(path)?)
            .bearer_auth(&self.session.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await.unwrap_or_default();
        if !status.is_success() {
            let reason = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| value["reason"].as_str().map(str::to_owned))
                .unwrap_or_else(|| match status.as_u16() {
                    404 | 405 => "relay_unsupported".to_owned(),
                    _ => "relay_error".to_owned(),
                });
            return Err(ConnectionRefusal {
                status: status.as_u16(),
                reason,
            }
            .into());
        }
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

// ── Local store ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionRole {
    Inviter,
    Acceptor,
}

/// One connection as this device remembers it. Fields added later must have
/// defaults; older builds ignore fields they do not know.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionRecord {
    pub connection_id: String,
    pub relay_url: String,
    pub local_device_id: String,
    pub role: ConnectionRole,
    /// `pending`, `active` or `revoked`, as last reported by the Relay.
    pub status: String,
    pub peer_user_id: String,
    pub peer_display_name: String,
    pub peer_device_id: String,
    pub peer_device_name: String,
    /// The peer key this device accepted while pairing. Unset until then.
    #[serde(default)]
    pub pinned_public_key: Option<String>,
    /// The peer key the Relay reported most recently.
    #[serde(default)]
    pub reported_public_key: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub confirmed_at: Option<i64>,
    #[serde(default)]
    pub revoked_at: Option<i64>,
    /// Set when this device stopped trusting the connection, e.g. `key_changed`.
    #[serde(default)]
    pub disabled_reason: Option<String>,
}

impl ConnectionRecord {
    fn from_relay(relay_url: &str, remote: &RelayConnection) -> Self {
        Self {
            connection_id: remote.connection_id.clone(),
            relay_url: relay_url.to_owned(),
            local_device_id: remote.local_device_id.clone(),
            role: remote.role,
            status: remote.status.clone(),
            peer_user_id: remote.peer.user_id.clone(),
            peer_display_name: remote.peer.display_name.clone(),
            peer_device_id: remote.peer.device_id.clone(),
            peer_device_name: remote.peer.device_name.clone(),
            pinned_public_key: None,
            reported_public_key: Some(remote.peer.public_key.clone()),
            created_at: remote.created_at,
            confirmed_at: remote.confirmed_at,
            revoked_at: remote.revoked_at,
            disabled_reason: None,
        }
    }

    /// Why the connection cannot carry requests now, if it cannot.
    pub fn unusable_reason(&self) -> Option<&str> {
        if let Some(reason) = self.disabled_reason.as_deref() {
            return Some(reason);
        }
        match self.status.as_str() {
            "active" if self.pinned_public_key.is_some() => None,
            "active" => Some("not_verified"),
            "pending" => Some("connection_pending"),
            _ => Some("connection_revoked"),
        }
    }

    /// The key for this connection, or why it cannot be used.
    pub fn connection_key(&self, device_secret: &[u8; 32]) -> Result<[u8; 32]> {
        if let Some(reason) = self.unusable_reason() {
            return Err(ConnectionUnusable {
                connection_id: self.connection_id.clone(),
                reason: reason.to_owned(),
            }
            .into());
        }
        let pinned = self
            .pinned_public_key
            .as_deref()
            .ok_or_else(|| anyhow!("connection has no pinned key"))?;
        derive_connection_key(
            device_secret,
            &super::encryption::parse_public_key(pinned)?,
            &self.connection_id,
        )
    }

    pub fn peer_fingerprint(&self) -> Option<String> {
        self.pinned_public_key
            .as_deref()
            .or(self.reported_public_key.as_deref())
            .and_then(|key| fingerprint_of(key).ok())
    }
}

/// A connection this device will not use, with the reason code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionUnusable {
    pub connection_id: String,
    pub reason: String,
}

impl std::fmt::Display for ConnectionUnusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "connection {} cannot be used: {}",
            self.connection_id, self.reason
        )
    }
}

impl std::error::Error for ConnectionUnusable {}

/// A stored entry that could not be read. It is kept as written.
#[derive(Debug, Clone, PartialEq)]
pub struct UnreadableConnection {
    pub connection_id: Option<String>,
    pub error: String,
    raw: Value,
}

#[derive(Debug, Clone, Default)]
pub struct StoredConnections {
    pub records: Vec<ConnectionRecord>,
    pub unreadable: Vec<UnreadableConnection>,
}

impl StoredConnections {
    /// The whole file could not be parsed, so nothing can be written to it.
    pub fn file_unreadable(&self) -> bool {
        self.unreadable
            .iter()
            .any(|entry| entry.connection_id.is_none() && entry.raw.is_null())
    }

    pub fn find(&self, connection_id: &str) -> Option<&ConnectionRecord> {
        self.records
            .iter()
            .find(|record| record.connection_id == connection_id)
    }

    fn upsert(&mut self, record: ConnectionRecord) {
        match self
            .records
            .iter_mut()
            .find(|existing| existing.connection_id == record.connection_id)
        {
            Some(existing) => *existing = record,
            None => self.records.push(record),
        }
    }
}

fn store_unreadable() -> anyhow::Error {
    ConnectionUnusable {
        connection_id: String::new(),
        reason: "store_unreadable".into(),
    }
    .into()
}

/// The connections file of one device: `connections.json` next to the
/// account session.
#[derive(Debug, Clone)]
pub struct ConnectionStore {
    path: PathBuf,
}

impl ConnectionStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn open_default() -> Result<Self> {
        Ok(Self::at(super::session_store::connections_file_path()?))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads every entry it can. A file that is not JSON at all is reported
    /// as one unreadable entry and left in place.
    pub fn load(&self) -> Result<StoredConnections> {
        let Some(file) = crate::remote_persistence::read_connection_store(&self.path)? else {
            return Ok(StoredConnections::default());
        };
        let entries = match file {
            Ok(value) => match value.get("connections").and_then(Value::as_array) {
                Some(entries) => entries.clone(),
                None => {
                    return Ok(StoredConnections {
                        records: Vec::new(),
                        unreadable: vec![UnreadableConnection {
                            connection_id: None,
                            error: "connections file has no connection list".into(),
                            raw: value,
                        }],
                    })
                }
            },
            Err(error) => {
                return Ok(StoredConnections {
                    records: Vec::new(),
                    unreadable: vec![UnreadableConnection {
                        connection_id: None,
                        error,
                        raw: Value::Null,
                    }],
                })
            }
        };
        let mut stored = StoredConnections::default();
        for entry in entries {
            match serde_json::from_value::<ConnectionRecord>(entry.clone()) {
                Ok(record) => stored.records.push(record),
                Err(error) => stored.unreadable.push(UnreadableConnection {
                    connection_id: entry["connectionId"].as_str().map(str::to_owned),
                    error: error.to_string(),
                    raw: entry,
                }),
            }
        }
        Ok(stored)
    }

    /// Writes readable records and keeps unreadable entries as they were. A
    /// file that could not be parsed at all is never overwritten.
    pub fn save(&self, stored: &StoredConnections) -> Result<()> {
        if stored.file_unreadable() {
            return Err(store_unreadable());
        }
        let mut entries = stored
            .records
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        entries.extend(
            stored
                .unreadable
                .iter()
                .filter(|entry| entry.connection_id.is_some() || entry.raw.is_object())
                .map(|entry| entry.raw.clone()),
        );
        crate::remote_persistence::write_connection_store(
            &self.path,
            &json!({ "version": STORE_VERSION, "connections": entries }),
        )
    }

    /// The record for `connection_id` if this device, signed in to
    /// `relay_url` as `local_device_id`, can use it now.
    pub fn usable(
        &self,
        connection_id: &str,
        relay_url: &str,
        local_device_id: &str,
    ) -> Result<ConnectionRecord> {
        let stored = self.load()?;
        if stored.file_unreadable() {
            return Err(store_unreadable());
        }
        if stored
            .unreadable
            .iter()
            .any(|entry| entry.connection_id.as_deref() == Some(connection_id))
        {
            return Err(ConnectionUnusable {
                connection_id: connection_id.to_owned(),
                reason: "record_unreadable".into(),
            }
            .into());
        }
        let record = stored
            .find(connection_id)
            .filter(|record| {
                record.local_device_id == local_device_id
                    && same_relay(&record.relay_url, relay_url)
            })
            .cloned()
            .ok_or_else(|| {
                anyhow::Error::from(ConnectionUnusable {
                    connection_id: connection_id.to_owned(),
                    reason: "connection_not_found".into(),
                })
            })?;
        if let Some(reason) = record.unusable_reason() {
            return Err(ConnectionUnusable {
                connection_id: connection_id.to_owned(),
                reason: reason.to_owned(),
            }
            .into());
        }
        Ok(record)
    }

    /// Record what a refused call revealed: a revoked connection or a changed
    /// key. Other refusals leave the record as it is.
    pub fn note_refusal(&self, connection_id: &str, reason: &str) -> Result<()> {
        let mut stored = self.load()?;
        let Some(record) = stored
            .records
            .iter_mut()
            .find(|record| record.connection_id == connection_id)
        else {
            return Ok(());
        };
        match reason {
            "connection_revoked" | "connection_not_found" => {
                if record.status == "revoked" {
                    return Ok(());
                }
                record.status = "revoked".into();
                record.revoked_at = Some(chrono::Utc::now().timestamp());
            }
            "connection_key_changed" => {
                if record.disabled_reason.as_deref() == Some("key_changed") {
                    return Ok(());
                }
                record.disabled_reason = Some("key_changed".into());
            }
            _ => return Ok(()),
        }
        self.save(&stored)
    }

    /// Merge the Relay's list for this device into the store. Status follows
    /// the Relay; pinned keys never change, and a different reported key
    /// disables the connection.
    pub fn reconcile(
        &self,
        relay_url: &str,
        local_device_id: &str,
        remote: &[RelayConnection],
    ) -> Result<StoredConnections> {
        let mut stored = self.load()?;
        // An entry this build cannot read keeps its id: it stays disabled
        // rather than being shadowed by a fresh, unpinned copy.
        let unreadable: Vec<String> = stored
            .unreadable
            .iter()
            .filter_map(|entry| entry.connection_id.clone())
            .collect();
        for remote in remote.iter().filter(|remote| {
            remote.local_device_id == local_device_id && !unreadable.contains(&remote.connection_id)
        }) {
            let mut record = stored
                .find(&remote.connection_id)
                .cloned()
                .unwrap_or_else(|| ConnectionRecord::from_relay(relay_url, remote));
            record.status = remote.status.clone();
            record.confirmed_at = remote.confirmed_at;
            record.revoked_at = remote.revoked_at;
            record.peer_display_name = remote.peer.display_name.clone();
            record.peer_device_name = remote.peer.device_name.clone();
            record.reported_public_key = Some(remote.peer.public_key.clone());
            if remote.revoked_reason.as_deref() == Some("connection_key_changed")
                || record
                    .pinned_public_key
                    .as_deref()
                    .is_some_and(|pinned| pinned != remote.peer.public_key)
            {
                record.disabled_reason = Some("key_changed".into());
            }
            stored.upsert(record);
        }
        self.save(&stored)?;
        Ok(stored)
    }
}

// ── Pairing ────────────────────────────────────────────────────────────

/// A logged-in device of one account, ready to manage its connections.
pub struct ConnectionsClient<'a> {
    pub relay_url: &'a str,
    pub session: &'a AccountSession,
    pub local_device_id: &'a str,
    pub store: &'a ConnectionStore,
}

/// An invite this device created, ready to send to the other person.
#[derive(Debug, Clone)]
pub struct CreatedConnectionInvite {
    pub invite_id: String,
    pub link: InviteLink,
    pub expires_at: i64,
}

/// What the accepting device shows before the person accepts.
#[derive(Debug, Clone)]
pub struct InvitePreview {
    pub inviter: RelayPeer,
    pub fingerprint: String,
    pub expires_at: i64,
}

impl ConnectionsClient<'_> {
    fn api(&self) -> RelayApi<'_> {
        RelayApi {
            relay_url: self.relay_url,
            session: self.session,
        }
    }

    pub fn local_fingerprint(&self) -> String {
        fingerprint(&device_crypto::public_key(&self.session.master_key))
    }

    pub async fn create_invite(
        &self,
        expires_in_seconds: Option<i64>,
    ) -> Result<CreatedConnectionInvite> {
        let body = match expires_in_seconds {
            Some(seconds) => json!({ "expires_in_seconds": seconds }),
            None => json!({}),
        };
        let created: CreatedInvite = serde_json::from_value(
            self.api()
                .send(
                    reqwest::Method::POST,
                    "/api/connections/invites",
                    Some(body),
                )
                .await?,
        )?;
        Ok(CreatedConnectionInvite {
            invite_id: created.invite_id,
            link: InviteLink {
                relay_url: self.relay_url.trim_end_matches('/').to_owned(),
                code: created.code,
                fingerprint: self.local_fingerprint().replace(' ', ""),
            },
            expires_at: created.expires_at,
        })
    }

    pub async fn list_invites(&self) -> Result<Vec<OpenInvite>> {
        Ok(serde_json::from_value(
            self.api()
                .send(reqwest::Method::GET, "/api/connections/invites", None)
                .await?,
        )?)
    }

    pub async fn withdraw_invite(&self, invite_id: &str) -> Result<()> {
        self.api()
            .send(
                reqwest::Method::DELETE,
                &format!(
                    "/api/connections/invites/{}",
                    urlencoding::encode(invite_id)
                ),
                None,
            )
            .await?;
        Ok(())
    }

    /// Reads the invite and checks the reported key against the link.
    pub async fn preview(&self, link: &InviteLink) -> Result<InvitePreview> {
        if !same_relay(&link.relay_url, self.relay_url) {
            return Err(PairingError::DifferentRelay.into());
        }
        let preview: RelayInvitePreview = serde_json::from_value(
            self.api()
                .send(
                    reqwest::Method::POST,
                    "/api/connections/invites/preview",
                    Some(json!({ "code": link.code })),
                )
                .await?,
        )?;
        let fingerprint = fingerprint_of(&preview.inviter.public_key)?;
        if !same_fingerprint(&fingerprint, &link.fingerprint) {
            return Err(PairingError::FingerprintMismatch.into());
        }
        Ok(InvitePreview {
            inviter: preview.inviter,
            fingerprint,
            expires_at: preview.expires_at,
        })
    }

    /// Accepts after the same checks as `preview`, and pins the inviter key.
    pub async fn accept(&self, link: &InviteLink) -> Result<ConnectionRecord> {
        // Refuse before the relay records a connection this device cannot keep.
        if self.store.load()?.file_unreadable() {
            return Err(store_unreadable());
        }
        let preview = self.preview(link).await?;
        let remote: RelayConnection = serde_json::from_value(
            self.api()
                .send(
                    reqwest::Method::POST,
                    "/api/connections/accept",
                    Some(json!({ "code": link.code })),
                )
                .await?,
        )?;
        if remote.peer.public_key != preview.inviter.public_key {
            return Err(PairingError::FingerprintMismatch.into());
        }
        let mut record = ConnectionRecord::from_relay(self.relay_url, &remote);
        record.pinned_public_key = Some(remote.peer.public_key.clone());
        let mut stored = self.store.load()?;
        stored.upsert(record.clone());
        self.store.save(&stored)?;
        Ok(record)
    }

    /// The Relay's view merged into the store.
    pub async fn list(&self) -> Result<StoredConnections> {
        let remote: Vec<RelayConnection> = serde_json::from_value(
            self.api()
                .send(reqwest::Method::GET, "/api/connections", None)
                .await?,
        )?;
        self.store
            .reconcile(self.relay_url, self.local_device_id, &remote)
    }

    /// Completes a pairing this device started. `expected_fingerprint` is the
    /// fingerprint the person checked; the key it names is pinned.
    pub async fn confirm(
        &self,
        connection_id: &str,
        expected_fingerprint: &str,
    ) -> Result<ConnectionRecord> {
        if self.store.load()?.file_unreadable() {
            return Err(store_unreadable());
        }
        let stored = self.list().await?;
        let record = stored
            .find(connection_id)
            .cloned()
            .ok_or_else(|| ConnectionRefusal {
                status: 404,
                reason: "connection_not_found".into(),
            })?;
        let reported = record
            .reported_public_key
            .clone()
            .ok_or_else(|| anyhow!("the relay reported no key for the other device"))?;
        if !same_fingerprint(&fingerprint_of(&reported)?, expected_fingerprint) {
            return Err(PairingError::FingerprintMismatch.into());
        }
        let remote: RelayConnection = serde_json::from_value(
            self.api()
                .send(
                    reqwest::Method::POST,
                    &format!(
                        "/api/connections/{}/confirm",
                        urlencoding::encode(connection_id)
                    ),
                    None,
                )
                .await?,
        )?;
        if remote.peer.public_key != reported {
            return Err(PairingError::FingerprintMismatch.into());
        }
        let mut record = record;
        record.status = remote.status;
        record.confirmed_at = remote.confirmed_at;
        record.pinned_public_key = Some(reported);
        let mut stored = self.store.load()?;
        stored.upsert(record.clone());
        self.store.save(&stored)?;
        Ok(record)
    }

    /// Revokes on the Relay, then marks the local record revoked.
    pub async fn revoke(&self, connection_id: &str) -> Result<()> {
        self.api()
            .send(
                reqwest::Method::DELETE,
                &format!("/api/connections/{}", urlencoding::encode(connection_id)),
                None,
            )
            .await?;
        let mut stored = self.store.load()?;
        if let Some(record) = stored
            .records
            .iter_mut()
            .find(|record| record.connection_id == connection_id)
        {
            record.status = "revoked".into();
            record.revoked_at = Some(chrono::Utc::now().timestamp());
        }
        self.store.save(&stored)
    }
}

// ── Overview for management surfaces ───────────────────────────────────

/// One connection as management surfaces show it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSummary {
    pub connection_id: String,
    pub role: ConnectionRole,
    pub status: String,
    pub usable: bool,
    /// Why the connection cannot carry requests, e.g. `key_changed`.
    pub unusable_reason: Option<String>,
    /// The inviting device still has to confirm after checking the fingerprint.
    pub awaiting_confirmation: bool,
    pub peer_user_id: String,
    pub peer_display_name: String,
    pub peer_device_id: String,
    pub peer_device_name: String,
    pub peer_fingerprint: Option<String>,
    pub created_at: i64,
    pub confirmed_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

impl From<&ConnectionRecord> for ConnectionSummary {
    fn from(record: &ConnectionRecord) -> Self {
        let unusable_reason = record.unusable_reason().map(str::to_owned);
        Self {
            connection_id: record.connection_id.clone(),
            role: record.role,
            status: record.status.clone(),
            usable: unusable_reason.is_none(),
            awaiting_confirmation: record.role == ConnectionRole::Inviter
                && record.status == "pending"
                && record.disabled_reason.is_none(),
            unusable_reason,
            peer_user_id: record.peer_user_id.clone(),
            peer_display_name: record.peer_display_name.clone(),
            peer_device_id: record.peer_device_id.clone(),
            peer_device_name: record.peer_device_name.clone(),
            peer_fingerprint: record.peer_fingerprint(),
            created_at: record.created_at,
            confirmed_at: record.confirmed_at,
            revoked_at: record.revoked_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnreadableSummary {
    pub connection_id: Option<String>,
    pub error: String,
}

/// Everything a connections page or `connections list` shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionsOverview {
    /// The relay can route between accounts.
    pub supported: bool,
    /// Set when the latest relay state could not be read; local records are shown.
    pub relay_error: Option<String>,
    pub local_device_id: String,
    pub local_fingerprint: String,
    pub connections: Vec<ConnectionSummary>,
    pub unreadable: Vec<UnreadableSummary>,
    pub invites: Vec<OpenInvite>,
}

impl ConnectionsClient<'_> {
    /// The relay's view merged into the store, or the local records alone
    /// when the relay cannot route between accounts or cannot be reached.
    pub async fn overview(&self, supported: bool) -> Result<ConnectionsOverview> {
        let local = self.store.load()?;
        let (stored, invites, relay_error) = if supported && local.file_unreadable() {
            // Nothing can be merged into a file that is kept as it is.
            (local, self.list_invites().await.unwrap_or_default(), None)
        } else if supported {
            match self.list().await {
                Ok(stored) => {
                    let invites = self.list_invites().await.unwrap_or_default();
                    (stored, invites, None)
                }
                Err(error) => (
                    self.store.load()?,
                    Vec::new(),
                    Some(
                        refusal_reason(&error)
                            .map(str::to_owned)
                            .unwrap_or_else(|| error.to_string()),
                    ),
                ),
            }
        } else {
            (local, Vec::new(), None)
        };
        Ok(ConnectionsOverview {
            supported,
            relay_error,
            local_device_id: self.local_device_id.to_owned(),
            local_fingerprint: self.local_fingerprint(),
            connections: stored
                .records
                .iter()
                .filter(|record| {
                    record.local_device_id == self.local_device_id
                        && same_relay(&record.relay_url, self.relay_url)
                })
                .map(ConnectionSummary::from)
                .collect(),
            unreadable: stored
                .unreadable
                .iter()
                .map(|entry| UnreadableSummary {
                    connection_id: entry.connection_id.clone(),
                    error: entry.error.clone(),
                })
                .collect(),
            invites: invites
                .into_iter()
                .filter(|invite| invite.device_id == self.local_device_id)
                .collect(),
        })
    }
}

// ── Request envelope ───────────────────────────────────────────────────

/// Encrypt one connection request. The connection id and command are inside
/// the ciphertext too, so the receiver can check them against the routing
/// fields the Relay reports.
pub fn seal_request(
    key: &[u8; 32],
    connection_id: &str,
    command: &str,
    args: &Value,
) -> Result<(String, String)> {
    let plaintext = json!({
        "cmd": "host_invoke",
        "connectionId": connection_id,
        "command": command,
        "args": args,
    })
    .to_string();
    super::encryption::encrypt_to_base64(key, &plaintext)
}

/// A decrypted request whose inner fields match the routing fields.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenedRequest {
    pub command: String,
    pub args: Value,
}

pub fn open_request(
    key: &[u8; 32],
    connection_id: &str,
    routed_command: &str,
    data: &str,
    nonce: &str,
) -> Result<OpenedRequest> {
    let plaintext = super::encryption::decrypt_from_base64(key, data, nonce)
        .map_err(|_| anyhow!("connection request could not be decrypted"))?;
    let value: Value = serde_json::from_str(&plaintext)?;
    if value["cmd"] != "host_invoke"
        || value["connectionId"] != connection_id
        || value["command"] != routed_command
    {
        bail!("connection request does not match its routing");
    }
    Ok(OpenedRequest {
        command: routed_command.to_owned(),
        args: value.get("args").cloned().unwrap_or(Value::Null),
    })
}

/// Encrypt an answer: `Ok(value)` or an error message.
pub fn seal_response(
    key: &[u8; 32],
    result: std::result::Result<Value, String>,
) -> Result<(String, String)> {
    let plaintext = match result {
        Ok(value) => json!({"resp": "host_invoke_result", "ok": true, "value": value}),
        Err(error) => json!({"resp": "host_invoke_result", "ok": false, "error": error}),
    }
    .to_string();
    super::encryption::encrypt_to_base64(key, &plaintext)
}

pub fn open_response(key: &[u8; 32], data: &str, nonce: &str) -> Result<Value> {
    let plaintext = super::encryption::decrypt_from_base64(key, data, nonce)
        .map_err(|_| anyhow!("connection answer could not be decrypted"))?;
    let value: Value = serde_json::from_str(&plaintext)?;
    if value["resp"] != "host_invoke_result" {
        bail!("connection answer is malformed");
    }
    if value["ok"] == true {
        Ok(value.get("value").cloned().unwrap_or(Value::Null))
    } else {
        bail!(
            "{}",
            value["error"]
                .as_str()
                .unwrap_or("the other device refused the request")
        )
    }
}

/// Base64 of a device public key derived from its secret.
pub fn local_public_key_base64(device_secret: &[u8; 32]) -> String {
    BASE64.encode(device_crypto::public_key(device_secret))
}

#[cfg(test)]
mod tests;
