//! Records cross-account tasks in this device's delegation audit log.
//!
//! Recording never fails a task: a log that cannot be written is reported in
//! this device's own log, and the task runs as its owner allowed.

use openbitfun_core::service::delegation_audit::{
    DelegationAuditEntry, DelegationAuditEvent, DelegationAuditLog,
};

use openbitfun_core::service::delegation::DelegationTrust;

use super::protocol::{DispatchJobState, DispatchSubmitRequest};

fn default_log() -> Option<DelegationAuditLog> {
    openbitfun_core::infrastructure::PathManager::new()
        .ok()
        .map(|path_manager| DelegationAuditLog::for_product_home(&path_manager))
}

/// The entry for `request`, or none when it did not come over a connection.
pub(super) fn entry(
    request: &DispatchSubmitRequest,
    event: DelegationAuditEvent,
    trust: DelegationTrust,
    external_agents: Vec<String>,
    state: Option<DispatchJobState>,
    detail: Option<&str>,
) -> Option<DelegationAuditEntry> {
    let connection_id = request.source_connection_id.as_ref()?;
    let delegation = request.delegation.as_ref()?;
    Some(DelegationAuditEntry {
        at: chrono::Utc::now().to_rfc3339(),
        event,
        job_id: request.job_id.clone(),
        connection_id: connection_id.clone(),
        // Who the requester is comes from the share grant (F002-11 Task 2).
        peer_user_id: String::new(),
        peer_display_name: String::new(),
        peer_device_id: String::new(),
        peer_device_name: String::new(),
        open_agent_id: delegation.open_agent_id.clone(),
        trust: trust.as_str().to_string(),
        external_agents,
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

/// Appends one event for a cross-account task to this device's log, with
/// the connection's trust level now and the external agents the task started.
pub(super) async fn record(
    request: &DispatchSubmitRequest,
    event: DelegationAuditEvent,
    state: Option<DispatchJobState>,
    detail: Option<&str>,
    external_agents: Vec<String>,
) {
    if request.source_connection_id.is_none() {
        return;
    }
    let trust = super::worker::delegation_trust(request).await;
    let Some(entry) = entry(request, event, trust, external_agents, state, detail) else {
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
