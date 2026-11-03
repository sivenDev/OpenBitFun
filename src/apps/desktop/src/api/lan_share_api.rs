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
use openbitfun_core::service::dispatch::{
    forget_shared_assistant, OutboundDispatchStore, SharedAssistantProblem,
};
use openbitfun_core::service::lan_share::dispatch_rpc::LanDispatchRpc;
use openbitfun_core::service::lan_share::protocol::RequestOpened;
use openbitfun_core::service::lan_share::remote_agents::SharedAssistantDirectory;
use openbitfun_core::service::lan_share::{
    delete_tenant, list_tenants, tenants_root, AddressOutcome, DiscoveredDevice, HostStatus,
    LanShareHost, LanShareOptions, RequestAnswer, ShareConfirmations, ShareDispatch,
    ShareRequestView, ShareSettings, SharedAssistant, SharedWithMeNow,
};
use openbitfun_services_core::dispatch_contract::DISPATCH_SOURCE_SHARE_KEY_ARG;
use serde::{Deserialize, Serialize};
use serde_json::Value;

static HOST: OnceLock<Result<LanShareHost, String>> = OnceLock::new();

/// Runs tasks for devices this one shares assistants with through the
/// executor that ships with the app.
struct DesktopShareDispatch;

#[async_trait::async_trait]
impl ShareDispatch for DesktopShareDispatch {
    async fn run(&self, command: &str, args: Value) -> Result<Value, String> {
        let share_key = args
            .get(DISPATCH_SOURCE_SHARE_KEY_ARG)
            .and_then(Value::as_str)
            .ok_or_else(|| "the sharing key of the calling device is unknown".to_string())?
            .to_string();
        super::dispatch_host::dispatch_for_share_key(command, args, &share_key)
            .await
            .map_err(|error| format!("{error:#}"))
    }
}

/// Starts the share host with the app. A host that cannot start is reported
/// by every command.
pub async fn start(events: Arc<dyn EventEmitter>) {
    let started = match PathManager::new() {
        Ok(path_manager) => {
            let mut options = LanShareOptions::for_product_home(&path_manager, events);
            options.dispatch = Some(Arc::new(DesktopShareDispatch));
            LanShareHost::start(options).await
        }
        Err(error) => Err(format!("resolve storage root: {error}")),
    };
    match &started {
        Ok(host) => register_shared_assistant_calls(host),
        Err(error) => log::warn!("LAN sharing unavailable: {}", error),
    }
    let _ = HOST.set(started);
}

/// Lets this device's agents call the assistants other devices share with
/// it, and observe again the calls an earlier process left running.
fn register_shared_assistant_calls(host: &LanShareHost) {
    let Some(coordinator) = openbitfun_core::agentic::coordination::get_global_coordinator() else {
        log::warn!("Shared assistants cannot be called: the conversation coordinator is not ready");
        return;
    };
    coordinator.set_remote_agent_port(Arc::new(SharedAssistantDirectory::new(
        Arc::new(host.clone()),
        Arc::new(LanDispatchRpc::new(host.clone())),
    )));
    coordinator.resume_remote_background_tasks();
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
    /// What the assistant is like; other devices see it when they look it up.
    #[serde(default)]
    pub description: Option<String>,
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
    pub description: Option<String>,
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
pub struct LanShareUnblockRequest {
    pub assistant_id: String,
    pub public_key: String,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareProbeAddressRequest {
    /// What the person typed: `ip` or `ip:port`.
    pub address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareAddDeviceRequest {
    pub address: String,
    /// The key of the device whose code the person compared.
    pub public_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareDeviceRequest {
    pub device_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanShareDeviceAddressRequest {
    pub device_id: String,
    pub address: String,
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
    let host = host()?;
    let share = host
        .share(
            &request.assistant_id,
            &request.name,
            request.settings,
            request.confirmations,
        )
        .await?;
    match &request.description {
        Some(description) => {
            host.set_assistant_description(&request.assistant_id, description)
                .await?;
            Ok(host
                .shares()
                .share_of(&request.assistant_id)
                .await
                .map_err(|error| error.to_string())?
                .unwrap_or(share))
        }
        None => Ok(share),
    }
}

/// Changes a share's settings, the name other devices see, or both.
#[tauri::command]
pub async fn lan_share_update(request: LanShareUpdateRequest) -> Result<SharedAssistant, String> {
    let host = host()?;
    if let Some(name) = &request.name {
        host.set_assistant_name(&request.assistant_id, name).await?;
    }
    if let Some(description) = &request.description {
        host.set_assistant_description(&request.assistant_id, description)
            .await?;
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

/// Lets a device the owner blocked ask for the assistant again.
#[tauri::command]
pub async fn lan_share_unblock(request: LanShareUnblockRequest) -> Result<(), String> {
    host()?
        .unblock(&request.assistant_id, &request.public_key)
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
pub async fn lan_share_shared_with_me() -> Result<Vec<SharedWithMeNow>, String> {
    host()?.shared_with_me_now().await
}

/// Removes an assistant shared with this device, and this device's records
/// of its conversations with it.
#[tauri::command]
pub async fn lan_share_remove(request: LanShareRemoveRequest) -> Result<(), String> {
    let host = host()?;
    let owner_device = host
        .shared_with_me()
        .await?
        .into_iter()
        .find(|shared| {
            shared.owner.public_key == request.owner_key
                && shared.assistant_id == request.assistant_id
        })
        .map(|shared| shared.owner.device_id);
    host.remove(&request.owner_key, &request.assistant_id)
        .await?;
    if let Some(owner_device) = owner_device {
        let store = OutboundDispatchStore::new(&path_manager()?);
        if let Err(error) =
            forget_shared_assistant(&store, &owner_device, &request.assistant_id).await
        {
            log::warn!(
                "Failed to delete conversations with a removed shared assistant: assistant_id={} error={}",
                request.assistant_id,
                error
            );
        }
    }
    Ok(())
}

/// Dispatch to assistants other devices share with this one, over the LAN
/// share channel.
pub(crate) fn dispatch_rpc() -> Result<LanDispatchRpc, String> {
    Ok(LanDispatchRpc::new(host()?.clone()))
}

/// The name this device shows for the assistant `assistant_id` of the
/// device `owner_device_id`, while it is shared with this device.
pub(crate) async fn shared_assistant_name(
    owner_device_id: &str,
    assistant_id: &str,
) -> Result<String, String> {
    host()?
        .shared_with_me()
        .await?
        .into_iter()
        .find(|shared| {
            shared.owner.device_id == owner_device_id && shared.assistant_id == assistant_id
        })
        .map(|shared| shared.name)
        .ok_or_else(|| SharedAssistantProblem::NotShared.to_string())
}

/// Says who answers at an address someone typed, for a device Bonjour
/// cannot see. Nothing is kept.
#[tauri::command]
pub async fn lan_share_probe_address(
    request: LanShareProbeAddressRequest,
) -> Result<AddressOutcome, String> {
    Ok(host()?.probe_address(&request.address).await)
}

/// Adds a device by address once its code was compared.
#[tauri::command]
pub async fn lan_share_add_device(
    request: LanShareAddDeviceRequest,
) -> Result<AddressOutcome, String> {
    host()?
        .add_device(&request.address, &request.public_key)
        .await
}

/// Forgets a device added by address.
#[tauri::command]
pub async fn lan_share_remove_device(request: LanShareDeviceRequest) -> Result<bool, String> {
    host()?.remove_device(&request.device_id).await
}

/// Moves a device added by address to another address.
#[tauri::command]
pub async fn lan_share_set_device_address(
    request: LanShareDeviceAddressRequest,
) -> Result<AddressOutcome, String> {
    host()?
        .set_device_address(&request.device_id, &request.address)
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

        let add: LanShareAddDeviceRequest = serde_json::from_value(serde_json::json!({
            "address": "172.17.9.12",
            "publicKey": "ab".repeat(32),
        }))
        .unwrap();
        assert_eq!(add.address, "172.17.9.12");
        let moved: LanShareDeviceAddressRequest = serde_json::from_value(
            serde_json::json!({ "deviceId": "d1", "address": "172.17.9.13:50000" }),
        )
        .unwrap();
        assert_eq!(moved.device_id, "d1");
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
