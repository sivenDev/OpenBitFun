//! Assistants this device opens to other devices and accounts.
//!
//! The commands manage the same device-local store the target CLI reads, so
//! the settings page and `openbitfun delegation` stay interchangeable.

use openbitfun_core::infrastructure::PathManager;
use openbitfun_core::service::delegation::{
    ConnectionLimits, ConnectionTrustStore, DelegationTaskKind, DelegationTrust, OpenAgent,
    OpenAgentRecord, OpenAgentStore,
};
use openbitfun_core::service::delegation_audit::{DelegationAuditLog, DelegationAuditRecord};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationOpenAgentRequest {
    pub id: String,
    /// The assistant to open, `assistant:<directory>`.
    pub agent_type: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub same_account_devices: Vec<String>,
    /// The owner confirmed that the assistant's tasks may call external
    /// agents with this device's own login or credentials.
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
    let path_manager =
        PathManager::new().map_err(|error| format!("resolve storage root: {error}"))?;
    let description = request
        .description
        .map(|description| description.trim().to_string())
        .filter(|description| !description.is_empty());
    let agent = OpenAgent {
        id: request.id.trim().to_string(),
        agent_type: request.agent_type,
        display_name: request.display_name.trim().to_string(),
        description,
        task_kinds: vec![DelegationTaskKind::Message],
        same_account_devices: request.same_account_devices,
        connections: Vec::new(),
    };
    OpenAgentStore::for_product_home(&path_manager)
        .open_assistant(
            &path_manager.assistant_workspace_base_dir(None),
            agent,
            request.usage_confirmed,
        )
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delegation_close_agent(request: DelegationCloseAgentRequest) -> Result<bool, String> {
    open_agent_store()?
        .close(&request.id)
        .await
        .map_err(|error| error.to_string())
}

/// Grants an opened assistant to a paired device of another account.
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
        .grant_connection(&request.id, &request.connection_id)
        .await
        .map_err(|error| error.to_string())
}

/// Withdraws an opened assistant from a connection.
#[tauri::command]
pub async fn delegation_revoke_connection(
    request: DelegationConnectionRevokeRequest,
) -> Result<bool, String> {
    open_agent_store()?
        .revoke_connection(&request.id, &request.connection_id)
        .await
        .map_err(|error| error.to_string())
}

/// Sets the limits of an opened assistant's grant to a connection.
#[tauri::command]
pub async fn delegation_set_connection_limits(
    request: DelegationConnectionLimitsRequest,
) -> Result<OpenAgent, String> {
    open_agent_store()?
        .set_connection_limits(&request.id, &request.connection_id, request.limits)
        .await
        .map_err(|error| error.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationConnectionTrustRequest {
    pub connection_id: String,
    pub level: DelegationTrust,
    /// The owner confirmed the risk of full trust; required for `full`.
    #[serde(default)]
    pub risk_confirmed: bool,
}

fn connection_trust_store() -> Result<ConnectionTrustStore, String> {
    let path_manager =
        PathManager::new().map_err(|error| format!("resolve storage root: {error}"))?;
    Ok(ConnectionTrustStore::for_product_home(&path_manager))
}

/// The connections trusted beyond normal; any other connection is normal.
#[tauri::command]
pub async fn delegation_list_connection_trust() -> Result<BTreeMap<String, DelegationTrust>, String>
{
    connection_trust_store()?
        .levels()
        .await
        .map_err(|error| error.to_string())
}

/// Sets how far tasks over a cross-account connection are trusted. Full
/// trust needs the owner's confirmation; it applies from each task's next
/// turn.
#[tauri::command]
pub async fn delegation_set_connection_trust(
    request: DelegationConnectionTrustRequest,
) -> Result<(), String> {
    if request.level == DelegationTrust::Full && !request.risk_confirmed {
        return Err("trusting a connection fully requires confirming its risk".to_string());
    }
    let known =
        openbitfun_core::service::remote_connect::connections::ConnectionStore::open_default()
            .and_then(|store| store.load())
            .map_err(|error| format!("read this device's connections: {error}"))?
            .find(&request.connection_id)
            .is_some();
    if !known {
        return Err(format!(
            "connection '{}' does not exist on this device",
            request.connection_id
        ));
    }
    connection_trust_store()?
        .set(&request.connection_id, request.level)
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
