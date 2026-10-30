//! Agents this device opens to other devices for delegated dispatch jobs.
//!
//! The commands manage the same device-local store the target CLI reads, so
//! the settings page and `openbitfun delegation` stay interchangeable.

use openbitfun_core::infrastructure::PathManager;
use openbitfun_core::service::delegation::{
    ConnectionLimits, DelegationTaskKind, ExternalAgentUsageNotice, OpenAgent, OpenAgentRecord,
    OpenAgentStore,
};
use openbitfun_core::service::delegation_audit::{DelegationAuditLog, DelegationAuditRecord};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationOpenAgentRequest {
    pub id: String,
    pub agent_type: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub task_kinds: Vec<DelegationTaskKind>,
    #[serde(default)]
    pub message_workspace: Option<String>,
    pub same_account_devices: Vec<String>,
    /// The owner confirmed the external agent's usage notice. Ignored for
    /// local agent modes; absent from older callers.
    #[serde(default)]
    pub usage_confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationCloseAgentRequest {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationConnectionGrantRequest {
    pub id: String,
    pub connection_id: String,
    /// The owner confirmed the external agent's usage notice again for this
    /// connection. Ignored for local agent modes.
    #[serde(default)]
    pub usage_confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationConnectionRevokeRequest {
    pub id: String,
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationConnectionLimitsRequest {
    pub id: String,
    pub connection_id: String,
    /// Unset fields remove that limit.
    #[serde(default, flatten)]
    pub limits: ConnectionLimits,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationAuditListRequest {
    /// Most recent entries to return; all when unset.
    #[serde(default)]
    pub limit: Option<usize>,
}

fn open_agent_store() -> Result<OpenAgentStore, String> {
    let path_manager =
        PathManager::new().map_err(|error| format!("resolve storage root: {error}"))?;
    Ok(OpenAgentStore::for_product_home(&path_manager))
}

#[tauri::command]
pub async fn delegation_list_open_agents() -> Result<Vec<OpenAgentRecord>, String> {
    open_agent_store()?
        .records()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delegation_open_agent(
    request: DelegationOpenAgentRequest,
) -> Result<OpenAgent, String> {
    let description = request
        .description
        .map(|description| description.trim().to_string())
        .filter(|description| !description.is_empty());
    let message_workspace = request
        .message_workspace
        .map(|workspace| workspace.trim().to_string())
        .filter(|workspace| !workspace.is_empty());
    if let Some(workspace) = &message_workspace {
        if !std::path::Path::new(workspace).is_dir() {
            return Err(format!("message workspace {workspace} is not a directory"));
        }
    }
    let agent = OpenAgent {
        id: request.id.trim().to_string(),
        agent_type: request.agent_type,
        display_name: request.display_name.trim().to_string(),
        description,
        task_kinds: request.task_kinds,
        message_workspace,
        same_account_devices: request.same_account_devices,
        connections: Vec::new(),
    };
    let store = open_agent_store()?;
    let opened = if ExternalAgentUsageNotice::for_agent_type(&agent.agent_type).is_some() {
        let executor = openbitfun_core::agentic::coordination::get_global_coordinator()
            .and_then(|coordinator| coordinator.external_agent_executor());
        store
            .open_external_agent(agent, request.usage_confirmed, executor.as_deref())
            .await
    } else {
        store.open_local_agent(agent).await
    };
    opened.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delegation_close_agent(request: DelegationCloseAgentRequest) -> Result<bool, String> {
    open_agent_store()?
        .close(&request.id)
        .await
        .map_err(|error| error.to_string())
}

/// Grants an opened agent to a paired device of another account.
#[tauri::command]
pub async fn delegation_grant_connection(
    request: DelegationConnectionGrantRequest,
) -> Result<OpenAgent, String> {
    let connections =
        openbitfun_core::service::remote_connect::connections::ConnectionStore::open_default()
            .map_err(|error| error.to_string())?;
    openbitfun_core::service::remote_connect::connection_rpc::grantable_connection(
        &connections,
        &request.connection_id,
    )
    .map_err(|error| error.to_string())?;
    open_agent_store()?
        .grant_connection(&request.id, &request.connection_id, request.usage_confirmed)
        .await
        .map_err(|error| error.to_string())
}

/// Withdraws an opened agent from a connection.
#[tauri::command]
pub async fn delegation_revoke_connection(
    request: DelegationConnectionRevokeRequest,
) -> Result<bool, String> {
    open_agent_store()?
        .revoke_connection(&request.id, &request.connection_id)
        .await
        .map_err(|error| error.to_string())
}

/// Sets the limits of an opened agent's grant to a connection.
#[tauri::command]
pub async fn delegation_set_connection_limits(
    request: DelegationConnectionLimitsRequest,
) -> Result<OpenAgent, String> {
    open_agent_store()?
        .set_connection_limits(&request.id, &request.connection_id, request.limits)
        .await
        .map_err(|error| error.to_string())
}

/// The tasks other accounts delegated to this device, oldest first;
/// unreadable lines are kept and flagged.
#[tauri::command]
pub async fn delegation_audit_list(
    request: DelegationAuditListRequest,
) -> Result<Vec<DelegationAuditRecord>, String> {
    let path_manager =
        PathManager::new().map_err(|error| format!("resolve storage root: {error}"))?;
    let log = DelegationAuditLog::for_product_home(&path_manager);
    let mut records = tokio::task::spawn_blocking(move || log.read())
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("read the delegation audit log: {error}"))?;
    if let Some(limit) = request.limit {
        records.drain(..records.len().saturating_sub(limit));
    }
    Ok(records)
}
