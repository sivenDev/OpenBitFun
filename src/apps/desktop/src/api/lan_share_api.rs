//! Sharing assistants on the local network: the owner's shares and
//! requests, and the Discover panel of the person asking.
//!
//! The share host lives in `openbitfun-core` (`service::lan_share`); these
//! commands expose it to the Web UI, which listens to its
//! `lan_share://...` events instead of polling. The host starts with the
//! app and listens only while an assistant is shared. Every command acts on
//! this device and stays local in Peer Device Mode.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use openbitfun_core::infrastructure::{events::EventEmitter, PathManager};
use openbitfun_core::service::lan_share::protocol::RequestOpened;
use openbitfun_core::service::lan_share::{
    delete_tenant, list_tenants, tenants_root, DiscoveredDevice, HostStatus, LanShareHost,
    LanShareOptions, RequestAnswer, ShareConfirmations, ShareRequestView, ShareSettings,
    SharedAssistant, SharedWithMe,
};
use serde::{Deserialize, Serialize};

static HOST: OnceLock<Result<LanShareHost, String>> = OnceLock::new();

/// Starts the share host with the app. A host that cannot start is reported
/// by every command.
pub async fn start(events: Arc<dyn EventEmitter>) {
    let started = match PathManager::new() {
        Ok(path_manager) => {
            LanShareHost::start(LanShareOptions::for_product_home(&path_manager, events)).await
        }
        Err(error) => Err(format!("resolve storage root: {error}")),
    };
    if let Err(error) = &started {
        log::warn!("LAN sharing unavailable: {}", error);
    }
    let _ = HOST.set(started);
}

fn host() -> Result<&'static LanShareHost, String> {
    match HOST.get() {
        Some(Ok(host)) => Ok(host),
        Some(Err(error)) => Err(format!(
            "sharing on the local network is unavailable: {error}"
        )),
        None => Err("sharing on the local network is still starting".to_string()),
    }
}

fn path_manager() -> Result<PathManager, String> {
    PathManager::new().map_err(|error| format!("resolve storage root: {error}"))
}

/// One shared assistant as its owner manages it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedShare {
    #[serde(flatten)]
    pub share: SharedAssistant,
    /// The assistant's workspace, to match it with the assistant list.
    pub root_path: PathBuf,
    /// Devices whose workspace under this assistant is still kept,
    /// including ones that no longer have access.
    pub tenant_device_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareOverview {
    pub status: HostStatus,
    pub shares: Vec<OwnedShare>,
    /// Why the shares cannot be read; nothing is shared until it is fixed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shares_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareShareRequest {
    pub assistant_id: String,
    pub name: String,
    pub settings: ShareSettings,
    #[serde(default)]
    pub confirmations: ShareConfirmations,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareUpdateRequest {
    pub assistant_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub settings: Option<ShareSettings>,
    #[serde(default)]
    pub confirmations: ShareConfirmations,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareAssistantRequest {
    pub assistant_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareRevokeRequest {
    pub assistant_id: String,
    pub public_key: String,
    #[serde(default)]
    pub block: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareDeleteTenantRequest {
    pub assistant_id: String,
    pub device_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareDecideRequest {
    pub request_id: String,
    pub answer: RequestAnswer,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareAskRequest {
    pub device_id: String,
    pub assistant_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareRemoveRequest {
    pub owner_key: String,
    pub assistant_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareDisplayNameRequest {
    pub name: String,
}

/// This device's sharing identity and the assistants it shares.
#[tauri::command]
pub async fn lan_share_overview() -> Result<LanShareOverview, String> {
    let host = host()?;
    let path_manager = path_manager()?;
    let (shares, shares_error) = match host.shares().shares().await {
        Ok(shares) => (shares, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    Ok(LanShareOverview {
        status: host.status().await,
        shares: owned_shares(
            &tenants_root(&path_manager),
            &path_manager.assistant_workspace_base_dir(None),
            shares,
        ),
        shares_error,
    })
}

fn owned_shares(
    tenants_root: &Path,
    assistants_base: &Path,
    shares: Vec<SharedAssistant>,
) -> Vec<OwnedShare> {
    let tenants = list_tenants(tenants_root);
    shares
        .into_iter()
        .map(|share| OwnedShare {
            root_path: assistants_base.join(&share.id),
            tenant_device_ids: tenants
                .iter()
                .filter(|tenant| tenant.assistant_id == share.id)
                .map(|tenant| tenant.device_id.clone())
                .collect(),
            share,
        })
        .collect()
}

#[tauri::command]
pub async fn lan_share_share(request: LanShareShareRequest) -> Result<SharedAssistant, String> {
    host()?
        .share(
            &request.assistant_id,
            &request.name,
            request.settings,
            request.confirmations,
        )
        .await
}

/// Changes a share's settings, the name other devices see, or both.
#[tauri::command]
pub async fn lan_share_update(request: LanShareUpdateRequest) -> Result<SharedAssistant, String> {
    let host = host()?;
    if let Some(name) = &request.name {
        host.set_assistant_name(&request.assistant_id, name).await?;
    }
    match request.settings {
        Some(settings) => {
            host.update(&request.assistant_id, settings, request.confirmations)
                .await
        }
        None => host
            .shares()
            .share_of(&request.assistant_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("assistant '{}' is not shared", request.assistant_id)),
    }
}

#[tauri::command]
pub async fn lan_share_stop(request: LanShareAssistantRequest) -> Result<bool, String> {
    host()?.stop(&request.assistant_id).await
}

#[tauri::command]
pub async fn lan_share_revoke(request: LanShareRevokeRequest) -> Result<(), String> {
    host()?
        .revoke(&request.assistant_id, &request.public_key, request.block)
        .await
}

/// Deletes what a device left under a shared assistant: its workspace,
/// `USER.md` and conversations. Only for a device without access.
#[tauri::command]
pub async fn lan_share_delete_tenant(request: LanShareDeleteTenantRequest) -> Result<bool, String> {
    let host = host()?;
    let share = host
        .shares()
        .share_of(&request.assistant_id)
        .await
        .map_err(|error| error.to_string())?;
    if share.is_some_and(|share| share.is_granted_to_device_id(&request.device_id)) {
        return Err("withdraw this device's access before deleting its data".to_string());
    }
    let path_manager = path_manager()?;
    let deleted = delete_tenant(&path_manager, &request.assistant_id, &request.device_id)
        .map_err(|error| format!("delete the device's data: {error}"))?;
    Ok(deleted)
}

#[tauri::command]
pub async fn lan_share_requests() -> Result<Vec<ShareRequestView>, String> {
    Ok(host()?.requests().await)
}

#[tauri::command]
pub async fn lan_share_decide(request: LanShareDecideRequest) -> Result<(), String> {
    host()?.decide(&request.request_id, request.answer).await
}

#[tauri::command]
pub async fn lan_share_discover_start() -> Result<Vec<DiscoveredDevice>, String> {
    host()?.discover_start().await
}

#[tauri::command]
pub async fn lan_share_discover_stop() -> Result<(), String> {
    host()?.discover_stop().await;
    Ok(())
}

#[tauri::command]
pub async fn lan_share_discover_list() -> Result<Vec<DiscoveredDevice>, String> {
    Ok(host()?.discovered())
}

#[tauri::command]
pub async fn lan_share_request(request: LanShareAskRequest) -> Result<RequestOpened, String> {
    host()?
        .request(&request.device_id, &request.assistant_id)
        .await
}

#[tauri::command]
pub async fn lan_share_shared_with_me() -> Result<Vec<SharedWithMe>, String> {
    host()?.shared_with_me().await
}

#[tauri::command]
pub async fn lan_share_remove(request: LanShareRemoveRequest) -> Result<(), String> {
    host()?
        .remove(&request.owner_key, &request.assistant_id)
        .await
}

#[tauri::command]
pub async fn lan_share_set_display_name(
    request: LanShareDisplayNameRequest,
) -> Result<HostStatus, String> {
    host()?.set_display_name(&request.name).await
}

/// Opens the system settings that decide whether this app may use the
/// local network, for when no sharing device can be found.
#[tauri::command]
pub async fn lan_share_open_network_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let (program, args) = (
        "open",
        ["x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork"].as_slice(),
    );
    #[cfg(target_os = "windows")]
    let (program, args) = (
        "cmd",
        ["/C", "start", "", "ms-settings:network-status"].as_slice(),
    );
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("open your desktop environment's network settings".to_string());
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let status = openbitfun_core::util::process_manager::create_command(program)
            .args(args)
            .status()
            .map_err(|error| format!("open network settings: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("open network settings: exited with {status}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_take_the_shapes_the_web_ui_sends() {
        let share: LanShareShareRequest = serde_json::from_value(serde_json::json!({
            "assistantId": "workspace-notes",
            "name": "Notes",
            "settings": { "trust": "normal", "sessionRetentionDays": 30, "maxConcurrentTasks": 3 },
            "confirmations": { "externalAgentUsage": true },
        }))
        .unwrap();
        assert_eq!(share.settings.session_retention_days, 30);
        assert!(share.confirmations.external_agent_usage);
        assert!(!share.confirmations.full_trust_risk);

        let update: LanShareUpdateRequest = serde_json::from_value(
            serde_json::json!({ "assistantId": "workspace", "name": "Writer" }),
        )
        .unwrap();
        assert!(update.settings.is_none());

        for (answer, expected) in [
            ("allow", RequestAnswer::Allow),
            ("decline", RequestAnswer::Decline),
            ("block", RequestAnswer::Block),
        ] {
            let decide: LanShareDecideRequest =
                serde_json::from_value(serde_json::json!({ "requestId": "r1", "answer": answer }))
                    .unwrap();
            assert_eq!(decide.answer, expected);
        }
        let revoke: LanShareRevokeRequest = serde_json::from_value(
            serde_json::json!({ "assistantId": "workspace", "publicKey": "ab" }),
        )
        .unwrap();
        assert!(!revoke.block);
    }

    #[test]
    fn a_share_lists_the_devices_whose_data_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let tenants = dir.path().join("delegation/tenants");
        let assistants = dir.path().join("personal_assistant");
        let device = "0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(tenants.join("workspace").join(device)).unwrap();
        std::fs::create_dir_all(tenants.join("workspace-other").join(device)).unwrap();
        let share: SharedAssistant = serde_json::from_value(serde_json::json!({
            "id": "workspace",
            "trust": "normal",
            "sessionRetentionDays": 7,
            "maxConcurrentTasks": 2,
            "sharedAt": "2026-11-02T00:00:00Z",
        }))
        .unwrap();
        let owned = owned_shares(&tenants, &assistants, vec![share]);
        assert_eq!(owned[0].tenant_device_ids, vec![device.to_string()]);
        assert_eq!(owned[0].root_path, assistants.join("workspace"));
    }
}
