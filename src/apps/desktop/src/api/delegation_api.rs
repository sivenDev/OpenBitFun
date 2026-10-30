//! Agents this device opens to other devices for delegated dispatch jobs.
//!
//! The commands manage the same device-local store the target CLI reads, so
//! the settings page and `openbitfun delegation` stay interchangeable.

use openbitfun_core::infrastructure::PathManager;
use openbitfun_core::service::delegation::{
    DelegationTaskKind, OpenAgent, OpenAgentRecord, OpenAgentStore,
};
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationCloseAgentRequest {
    pub id: String,
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
    open_agent_store()?
        .open_local_agent(OpenAgent {
            id: request.id.trim().to_string(),
            agent_type: request.agent_type,
            display_name: request.display_name.trim().to_string(),
            description,
            task_kinds: request.task_kinds,
            message_workspace,
            same_account_devices: request.same_account_devices,
        })
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
