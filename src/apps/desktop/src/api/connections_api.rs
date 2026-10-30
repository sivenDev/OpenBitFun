//! Connections settings page: pair this device with devices of other
//! accounts, and list or revoke those connections.
//!
//! Every command acts for this device's own account identity, so the
//! registry declares them `ControllerLocal`. Errors are a stable reason code
//! (for example `fingerprint_mismatch`) when one exists, so the page can
//! explain them; otherwise the message.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use openbitfun_core::service::remote_connect::connection_rpc::{
    authenticated_device_id, ConnectionRpc,
};
use openbitfun_core::service::remote_connect::connections::{
    refusal_reason, ConnectionStore, ConnectionSummary, ConnectionsClient, ConnectionsOverview,
    InviteLink,
};
use openbitfun_core::service::remote_connect::{AccountClient, AccountSession};

use super::remote_connect_api::{
    account_context_generation, account_context_is_current, read_account_context_for_generation,
};

struct SignedIn {
    session: AccountSession,
    relay_url: String,
    device_id: String,
    supported: bool,
    store: ConnectionStore,
}

impl SignedIn {
    fn client(&self) -> ConnectionsClient<'_> {
        ConnectionsClient {
            relay_url: &self.relay_url,
            session: &self.session,
            local_device_id: &self.device_id,
            store: &self.store,
        }
    }

    fn require_support(&self) -> Result<(), String> {
        if self.supported {
            Ok(())
        } else {
            Err("relay_unsupported".into())
        }
    }
}

async fn signed_in() -> Result<(SignedIn, u64), String> {
    let generation = account_context_generation();
    let (session, relay_url) = read_account_context_for_generation(generation).await?;
    let device_id = authenticated_device_id(&session).map_err(|error| error.to_string())?;
    let capabilities = AccountClient::new()
        .relay_capabilities(&relay_url)
        .await
        .unwrap_or_default();
    let store = ConnectionStore::open_default().map_err(|error| error.to_string())?;
    Ok((
        SignedIn {
            session,
            relay_url,
            device_id,
            supported: ConnectionRpc::relay_supports_connections(&capabilities),
            store,
        },
        generation,
    ))
}

fn reason(error: anyhow::Error) -> String {
    refusal_reason(&error)
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string())
}

fn still_current(generation: u64) -> Result<(), String> {
    if account_context_is_current(generation) {
        Ok(())
    } else {
        Err("account context changed".into())
    }
}

#[tauri::command]
pub async fn connections_overview() -> Result<ConnectionsOverview, String> {
    let (account, generation) = signed_in().await?;
    let overview = account
        .client()
        .overview(account.supported)
        .await
        .map_err(reason)?;
    still_current(generation)?;
    Ok(overview)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInviteRequest {
    #[serde(default)]
    pub expires_in_seconds: Option<i64>,
}

#[tauri::command]
pub async fn connections_create_invite(request: CreateInviteRequest) -> Result<Value, String> {
    let (account, generation) = signed_in().await?;
    account.require_support()?;
    let client = account.client();
    let invite = client
        .create_invite(request.expires_in_seconds)
        .await
        .map_err(reason)?;
    still_current(generation)?;
    Ok(json!({
        "inviteId": invite.invite_id,
        "link": invite.link.to_string(),
        "fingerprint": client.local_fingerprint(),
        "expiresAt": invite.expires_at,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteLinkRequest {
    pub link: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitePreviewView {
    pub inviter_user_id: String,
    pub inviter_display_name: String,
    pub inviter_device_id: String,
    pub inviter_device_name: String,
    /// Already checked against the fingerprint in the link.
    pub fingerprint: String,
    pub expires_at: i64,
}

#[tauri::command]
pub async fn connections_preview_invite(
    request: InviteLinkRequest,
) -> Result<InvitePreviewView, String> {
    let (account, generation) = signed_in().await?;
    account.require_support()?;
    let link = InviteLink::parse(&request.link).map_err(|_| "invalid_invite_link".to_string())?;
    let preview = account.client().preview(&link).await.map_err(reason)?;
    still_current(generation)?;
    Ok(InvitePreviewView {
        inviter_user_id: preview.inviter.user_id,
        inviter_display_name: preview.inviter.display_name,
        inviter_device_id: preview.inviter.device_id,
        inviter_device_name: preview.inviter.device_name,
        fingerprint: preview.fingerprint,
        expires_at: preview.expires_at,
    })
}

#[tauri::command]
pub async fn connections_accept_invite(
    request: InviteLinkRequest,
) -> Result<ConnectionSummary, String> {
    let (account, generation) = signed_in().await?;
    account.require_support()?;
    let link = InviteLink::parse(&request.link).map_err(|_| "invalid_invite_link".to_string())?;
    still_current(generation)?;
    let record = account.client().accept(&link).await.map_err(reason)?;
    log::info!("Connection accepted: connection={}", record.connection_id);
    Ok(ConnectionSummary::from(&record))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmConnectionRequest {
    pub connection_id: String,
    pub fingerprint: String,
}

#[tauri::command]
pub async fn connections_confirm(
    request: ConfirmConnectionRequest,
) -> Result<ConnectionSummary, String> {
    let (account, generation) = signed_in().await?;
    account.require_support()?;
    still_current(generation)?;
    let record = account
        .client()
        .confirm(&request.connection_id, &request.fingerprint)
        .await
        .map_err(reason)?;
    log::info!("Connection confirmed: connection={}", record.connection_id);
    Ok(ConnectionSummary::from(&record))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionIdRequest {
    pub connection_id: String,
}

#[tauri::command]
pub async fn connections_revoke(request: ConnectionIdRequest) -> Result<(), String> {
    let (account, generation) = signed_in().await?;
    account.require_support()?;
    still_current(generation)?;
    account
        .client()
        .revoke(&request.connection_id)
        .await
        .map_err(reason)?;
    log::info!("Connection revoked: connection={}", request.connection_id);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteIdRequest {
    pub invite_id: String,
}

#[tauri::command]
pub async fn connections_withdraw_invite(request: InviteIdRequest) -> Result<(), String> {
    let (account, generation) = signed_in().await?;
    account.require_support()?;
    still_current(generation)?;
    account
        .client()
        .withdraw_invite(&request.invite_id)
        .await
        .map_err(reason)
}
