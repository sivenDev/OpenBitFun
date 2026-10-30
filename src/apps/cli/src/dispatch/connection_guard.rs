//! Limits on what one connection may run with one open agent, and the guard
//! that stops a cross-account job when its owner withdraws it.
//!
//! The owner sets a connection's limits on its grant. The number of tasks
//! running at once is checked when a task is submitted; the length of a turn,
//! the grant, and the connection itself are checked while the turn runs, so
//! a revoked connection or grant stops work already in progress.

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use openbitfun_core::service::delegation::{ConnectionLimits, DelegationTaskKind, OpenAgentStore};
use openbitfun_core::service::remote_connect::connections::ConnectionStore;

use super::protocol::{DispatchDelegationTaskKind, DispatchSubmitRequest};
use super::store::{DispatchJobRecord, DispatchStore};

/// How often a running cross-account turn checks its guard.
pub(super) const GUARD_INTERVAL: Duration = Duration::from_secs(1);
/// How often the guard refreshes this device's connection records from the
/// Relay, so a revocation by the other side also stops running work.
const RELAY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// What the guard needs to read the Relay's view of this device's
/// connections.
pub(super) struct RelayRefresh {
    pub(super) relay_url: String,
    pub(super) session: openbitfun_core::service::remote_connect::AccountSession,
    pub(super) device_id: String,
}

impl RelayRefresh {
    /// The signed-in account of this device, if any.
    pub(super) fn signed_in() -> Option<Self> {
        use openbitfun_core::service::remote_connect::{connection_rpc, session_store};
        let loaded = session_store::load_session_detailed().ok().flatten()?;
        let session = openbitfun_core::service::remote_connect::AccountSession::new(
            loaded.token,
            loaded.user_id,
            loaded.master_key,
        );
        let device_id = connection_rpc::authenticated_device_id(&session).ok()?;
        Some(Self {
            relay_url: loaded.relay_url,
            session,
            device_id,
        })
    }
}

/// Why a running cross-account job stopped, as its requester reads it.
pub(super) const GRANT_WITHDRAWN: &str =
    "The owner of the target device withdrew this connection's access to the agent";
pub(super) const CONNECTION_REVOKED: &str = "The connection to the target device was revoked";

fn task_kind(kind: DispatchDelegationTaskKind) -> DelegationTaskKind {
    match kind {
        DispatchDelegationTaskKind::Message => DelegationTaskKind::Message,
        DispatchDelegationTaskKind::Coding => DelegationTaskKind::Coding,
    }
}

/// The limits the owner set on `open_agent_id` for `connection_id`.
async fn limits_for(
    grants: &OpenAgentStore,
    connection_id: &str,
    open_agent_id: &str,
) -> Result<ConnectionLimits> {
    Ok(grants
        .granted_to_connection(connection_id)
        .await
        .context("read agents opened to other devices")?
        .iter()
        .find(|agent| agent.id == open_agent_id)
        .and_then(|agent| agent.connection_grant(connection_id))
        .map(|grant| grant.limits)
        .unwrap_or_default())
}

/// Refuses a new cross-account task, or a new turn of one, when the
/// connection already runs as many tasks with this agent as its owner
/// allows. Hold [`DispatchStore::lock_connection_admission`] from this check
/// until the job or turn is queued.
pub(super) async fn ensure_within_limits(
    store: &DispatchStore,
    grants: &OpenAgentStore,
    request: &DispatchSubmitRequest,
) -> Result<()> {
    let (Some(connection_id), Some(delegation)) =
        (&request.source_connection_id, &request.delegation)
    else {
        return Ok(());
    };
    let limits = limits_for(grants, connection_id, &delegation.open_agent_id).await?;
    let Some(max) = limits.max_concurrent_jobs else {
        return Ok(());
    };
    let running = store
        .list_jobs()?
        .into_iter()
        .filter(|job| !job.state.is_terminal() && job.job_id != request.job_id)
        .filter(|job| {
            store.load_job(&job.job_id).is_ok_and(|record| {
                record.request.source_connection_id.as_deref() == Some(connection_id.as_str())
                    && record
                        .request
                        .delegation
                        .as_ref()
                        .is_some_and(|other| other.open_agent_id == delegation.open_agent_id)
            })
        })
        .count();
    if running >= max as usize {
        bail!(
            "the target device runs at most {max} task(s) at a time for this connection; try again when one finishes"
        );
    }
    Ok(())
}

/// Watches one turn of a cross-account job.
pub(super) struct ConnectionGuard {
    grants: OpenAgentStore,
    connections: ConnectionStore,
    connection_id: String,
    open_agent_id: String,
    kind: DelegationTaskKind,
    started: Instant,
    turn_limit: Option<Duration>,
    relay: Option<RelayRefresh>,
    last_refresh: std::sync::Mutex<Instant>,
}

impl ConnectionGuard {
    /// The guard for `job`, or none when it did not come over a connection.
    pub(super) async fn for_job(
        job: &DispatchJobRecord,
        grants: OpenAgentStore,
        connections: ConnectionStore,
        relay: Option<RelayRefresh>,
    ) -> Result<Option<Self>> {
        let (Some(connection_id), Some(delegation)) =
            (&job.request.source_connection_id, &job.request.delegation)
        else {
            return Ok(None);
        };
        let limits = limits_for(&grants, connection_id, &delegation.open_agent_id).await?;
        Ok(Some(Self {
            grants,
            connections,
            connection_id: connection_id.clone(),
            open_agent_id: delegation.open_agent_id.clone(),
            kind: task_kind(delegation.task_kind),
            started: Instant::now(),
            relay,
            last_refresh: std::sync::Mutex::new(Instant::now()),
            turn_limit: limits
                .max_job_minutes
                .map(|minutes| Duration::from_secs(u64::from(minutes) * 60)),
        }))
    }

    /// Brings this device's records up to date with the Relay now and then;
    /// a Relay that cannot be reached changes nothing.
    async fn refresh_from_relay(&self, now: Instant) {
        let Some(relay) = &self.relay else {
            return;
        };
        {
            let mut last = self
                .last_refresh
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if now.duration_since(*last) < RELAY_REFRESH_INTERVAL {
                return;
            }
            *last = now;
        }
        let client = openbitfun_core::service::remote_connect::connections::ConnectionsClient {
            relay_url: &relay.relay_url,
            session: &relay.session,
            local_device_id: &relay.device_id,
            store: &self.connections,
        };
        if let Err(error) = client.list().await {
            tracing::debug!("Connection refresh unavailable for a cross-account job: {error:#}");
        }
    }

    /// Why the turn must stop now, if it must.
    pub(super) async fn stop_reason(&self, now: Instant) -> Option<String> {
        match self
            .grants
            .authorize_connection(&self.connection_id, &self.open_agent_id, self.kind)
            .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return Some(GRANT_WITHDRAWN.to_string()),
            // An unreadable list is not a withdrawal; the next check retries.
            Err(error) => tracing::warn!("Delegation grant check unavailable: {error}"),
        }
        self.refresh_from_relay(now).await;
        // A file or record that cannot be read is not a revocation either.
        if let Ok(stored) = self.connections.load() {
            let unreadable = stored.file_unreadable()
                || stored.unreadable.iter().any(|entry| {
                    entry.connection_id.as_deref() == Some(self.connection_id.as_str())
                });
            let revoked = !unreadable
                && stored.find(&self.connection_id).is_none_or(|record| {
                    record.status == "revoked" || record.disabled_reason.is_some()
                });
            if revoked {
                return Some(CONNECTION_REVOKED.to_string());
            }
        }
        if let Some(limit) = self.turn_limit {
            if now.duration_since(self.started) >= limit {
                return Some(format!(
                    "The task ran longer than the {}-minute limit the owner of the target device set",
                    limit.as_secs() / 60
                ));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::protocol::{
        DispatchApprovalPolicy, DispatchDelegation, DispatchDelegationSource,
        DISPATCH_PROTOCOL_VERSION,
    };
    use openbitfun_core::service::delegation::OpenAgent;

    fn request(job_id: &str, connection_id: &str) -> DispatchSubmitRequest {
        DispatchSubmitRequest {
            protocol_version: DISPATCH_PROTOCOL_VERSION,
            job_id: job_id.to_string(),
            session_id: format!("session-{job_id}"),
            workspace_path: "/tmp".to_string(),
            agent_type: "agentic".to_string(),
            prompt: "hello".to_string(),
            approval_policy: DispatchApprovalPolicy::Remote,
            model: None,
            reasoning_preset: None,
            title: None,
            attachments: Vec::new(),
            setup_audit: Vec::new(),
            delegation: Some(DispatchDelegation {
                open_agent_id: "helper".to_string(),
                task_kind: DispatchDelegationTaskKind::Message,
                source: DispatchDelegationSource::default(),
            }),
            source_device_id: None,
            source_connection_id: Some(connection_id.to_string()),
        }
    }

    async fn grants(dir: &std::path::Path, limits: ConnectionLimits) -> OpenAgentStore {
        let grants = OpenAgentStore::new(dir.join("open-agents.json"));
        grants
            .open(OpenAgent {
                id: "helper".into(),
                agent_type: "agentic".into(),
                display_name: "Helper".into(),
                description: None,
                task_kinds: vec![DelegationTaskKind::Message],
                message_workspace: None,
                same_account_devices: Vec::new(),
                connections: Vec::new(),
            })
            .await
            .unwrap();
        grants
            .grant_connection("helper", "conn-1", false)
            .await
            .unwrap();
        grants
            .set_connection_limits("helper", "conn-1", limits)
            .await
            .unwrap();
        grants
    }

    fn connections(dir: &std::path::Path, status: &str) -> ConnectionStore {
        let store = ConnectionStore::at(dir.join("connections.json"));
        std::fs::write(
            store.path(),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "connections": [{
                    "connectionId": "conn-1", "relayUrl": "https://relay", "localDeviceId": "me",
                    "role": "inviter", "status": status, "peerUserId": "2002",
                    "peerDisplayName": "Bea", "peerDeviceId": "device-b",
                    "peerDeviceName": "B1 Desktop", "pinnedPublicKey": "key", "createdAt": 1,
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        store
    }

    #[tokio::test]
    async fn a_connection_runs_only_as_many_tasks_as_its_owner_allows() {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
        let grants = grants(
            dir.path(),
            ConnectionLimits {
                max_concurrent_jobs: Some(1),
                max_job_minutes: None,
            },
        )
        .await;
        ensure_within_limits(&store, &grants, &request("delegation-1", "conn-1"))
            .await
            .unwrap();
        let first = request("delegation-1", "conn-1");
        store
            .create_job_with_intent(first.clone(), first, "one".to_string())
            .unwrap();
        let error = ensure_within_limits(&store, &grants, &request("delegation-2", "conn-1"))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("at most 1 task"), "{error}");
        // A retried submission of the running job is not a second task.
        ensure_within_limits(&store, &grants, &request("delegation-1", "conn-1"))
            .await
            .unwrap();
        // Another connection has its own count.
        ensure_within_limits(&store, &grants, &request("delegation-3", "conn-2"))
            .await
            .unwrap();
        // A new turn of a finished task over the same connection is counted
        // like a new task while another one runs.
        let mut finished = request("delegation-old", "conn-1");
        finished.prompt = "earlier".to_string();
        let error = ensure_within_limits(&store, &grants, &finished)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("at most 1 task"), "{error}");
        // Admissions of one connection are serialized by a lock that is
        // released with its guard.
        drop(store.lock_connection_admission("conn-1").unwrap());
        drop(store.lock_connection_admission("conn-1").unwrap());
        // A finished task frees its place.
        store
            .mark_state(
                "delegation-1",
                crate::dispatch::protocol::DispatchJobState::Succeeded,
                None,
                None,
            )
            .unwrap();
        ensure_within_limits(&store, &grants, &request("delegation-2", "conn-1"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn running_turns_stop_on_withdrawal_revocation_or_time_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
        let grants = grants(
            dir.path(),
            ConnectionLimits {
                max_concurrent_jobs: None,
                max_job_minutes: Some(5),
            },
        )
        .await;
        let job = request("delegation-1", "conn-1");
        store
            .create_job_with_intent(job.clone(), job, "one".to_string())
            .unwrap();
        let record = store.load_job("delegation-1").unwrap();
        let guard =
            ConnectionGuard::for_job(
            &record,
            grants.clone(),
            connections(dir.path(), "active"),
            None,
        )
                .await
                .unwrap()
                .expect("a connection job is guarded");
        let now = guard.started;
        assert_eq!(guard.stop_reason(now).await, None);
        let late = guard
            .stop_reason(now + Duration::from_secs(5 * 60))
            .await
            .unwrap();
        assert!(late.contains("5-minute limit"), "{late}");

        let revoked =
            ConnectionGuard::for_job(
            &record,
            grants.clone(),
            connections(dir.path(), "revoked"),
            None,
        )
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            revoked.stop_reason(now).await.as_deref(),
            Some(CONNECTION_REVOKED)
        );

        grants.revoke_connection("helper", "conn-1").await.unwrap();
        assert_eq!(
            guard.stop_reason(now).await.as_deref(),
            Some(GRANT_WITHDRAWN)
        );

        // Jobs that did not come over a connection are not guarded.
        let mut local = request("delegation-local", "conn-1");
        local.source_connection_id = None;
        store
            .create_job_with_intent(local.clone(), local, "local".to_string())
            .unwrap();
        let local = store.load_job("delegation-local").unwrap();
        assert!(
            ConnectionGuard::for_job(&local, grants, connections(dir.path(), "active"), None)
                .await
                .unwrap()
                .is_none()
        );
    }
}
