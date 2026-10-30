//! Records cross-account tasks in this device's delegation audit log.
//!
//! Recording never fails a task: a log that cannot be written is reported in
//! this device's own log, and the task runs as its owner allowed.

use openbitfun_core::service::delegation_audit::{
    DelegationAuditEntry, DelegationAuditEvent, DelegationAuditLog,
};
use openbitfun_core::service::remote_connect::connections::ConnectionStore;

use super::protocol::{DispatchDelegationTaskKind, DispatchJobState, DispatchSubmitRequest};

fn default_log() -> Option<DelegationAuditLog> {
    openbitfun_core::infrastructure::PathManager::new()
        .ok()
        .map(|path_manager| DelegationAuditLog::for_product_home(&path_manager))
}

/// The entry for `request`, or none when it did not come over a connection.
pub(super) fn entry(
    request: &DispatchSubmitRequest,
    connections: Option<&ConnectionStore>,
    event: DelegationAuditEvent,
    state: Option<DispatchJobState>,
    detail: Option<&str>,
) -> Option<DelegationAuditEntry> {
    let connection_id = request.source_connection_id.as_ref()?;
    let delegation = request.delegation.as_ref()?;
    let peer = connections
        .and_then(|store| store.load().ok())
        .and_then(|stored| stored.find(connection_id).cloned());
    Some(DelegationAuditEntry {
        at: chrono::Utc::now().to_rfc3339(),
        event,
        job_id: request.job_id.clone(),
        connection_id: connection_id.clone(),
        peer_user_id: peer
            .as_ref()
            .map(|p| p.peer_user_id.clone())
            .unwrap_or_default(),
        peer_display_name: peer
            .as_ref()
            .map(|p| p.peer_display_name.clone())
            .unwrap_or_default(),
        peer_device_id: peer
            .as_ref()
            .map(|p| p.peer_device_id.clone())
            .unwrap_or_default(),
        peer_device_name: peer
            .as_ref()
            .map(|p| p.peer_device_name.clone())
            .unwrap_or_default(),
        open_agent_id: delegation.open_agent_id.clone(),
        task_kind: match delegation.task_kind {
            DispatchDelegationTaskKind::Message => "message",
            DispatchDelegationTaskKind::Coding => "coding",
        }
        .to_string(),
        state: state.map(|state| {
            match state {
                DispatchJobState::Queued => "queued",
                DispatchJobState::Running => "running",
                DispatchJobState::Succeeded => "succeeded",
                DispatchJobState::Failed => "failed",
                DispatchJobState::Cancelled => "cancelled",
            }
            .to_string()
        }),
        detail: detail.map(|detail| detail.chars().take(500).collect()),
    })
}

/// Appends one event for a cross-account task to this device's log.
pub(super) fn record(
    request: &DispatchSubmitRequest,
    event: DelegationAuditEvent,
    state: Option<DispatchJobState>,
    detail: Option<&str>,
) {
    let connections = ConnectionStore::open_default().ok();
    let Some(entry) = entry(request, connections.as_ref(), event, state, detail) else {
        return;
    };
    let Some(log) = default_log() else {
        tracing::warn!("Delegation audit log unavailable: no product home");
        return;
    };
    if let Err(error) = log.append(&entry) {
        tracing::warn!("Failed to record a cross-account task in the audit log: {error}");
    }
}
