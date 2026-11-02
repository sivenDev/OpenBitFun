//! The delegation audit log of this device: what other devices asked its
//! assistants to do. Sharing assistants is managed by the LAN share commands.

use openbitfun_core::infrastructure::PathManager;
use openbitfun_core::service::delegation_audit::{DelegationAuditLog, DelegationAuditRecord};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationAuditListRequest {
    /// Most recent entries to return; all when unset.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The tasks other devices delegated to this device, oldest first;
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
