//! Limits on what one connection may run with one open agent, and the guard
//! that stops a cross-account job when its owner withdraws it.
//!
//! The owner sets a connection's limits on its grant. A task over the number
//! of tasks running at once is accepted and waits, queued, until one of them
//! ends; waiting tasks start in the order they began to wait. The length of a
//! turn, the grant, and the connection itself are checked while the turn
//! runs, so a revoked connection or grant stops work already in progress.

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use openbitfun_core::service::delegation::{ConnectionLimits, OpenAgentStore};
use openbitfun_core::service::remote_connect::connections::ConnectionStore;

use super::protocol::{DispatchHold, DispatchHoldReason, DispatchJobState, DispatchSubmitRequest};
use super::store::{DispatchJobRecord, DispatchStore, WorkspaceLock};

/// How often a running cross-account turn checks its guard.
pub(super) const GUARD_INTERVAL: Duration = Duration::from_secs(1);
/// How often a queued task looks for a free place.
const PLACE_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_secs(2)
};
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

/// Whether two requests run the same open agent over the same connection.
fn same_place(request: &DispatchSubmitRequest, connection_id: &str, open_agent_id: &str) -> bool {
    request.source_connection_id.as_deref() == Some(connection_id)
        && request
            .delegation
            .as_ref()
            .is_some_and(|delegation| delegation.open_agent_id == open_agent_id)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// Whether `job_id`, waiting since `since_ms`, may start now: the tasks
/// running with this agent over this connection, and those that began to
/// wait before it, leave a place under `max`.
fn has_place(
    store: &DispatchStore,
    job_id: &str,
    since_ms: u64,
    connection_id: &str,
    open_agent_id: &str,
    max: u32,
) -> Result<bool> {
    let mut ahead = 0usize;
    for job in store.list_jobs()? {
        if job.job_id == job_id || job.state.is_terminal() {
            continue;
        }
        let Ok(record) = store.load_job(&job.job_id) else {
            continue;
        };
        if !same_place(&record.request, connection_id, open_agent_id) {
            continue;
        }
        let counts = match job.state {
            DispatchJobState::Running => true,
            // A task that waited longer starts first.
            DispatchJobState::Queued => store
                .read_hold(&job.job_id)
                .is_some_and(|hold| (hold.since_ms, job.job_id.as_str()) < (since_ms, job_id)),
            _ => false,
        };
        ahead += usize::from(counts);
    }
    Ok(ahead < max as usize)
}

/// Waits until the cross-account task `job_id` may run under its
/// connection's limit, recording why it waits meanwhile. Returns the
/// connection's admission lock, which the caller holds until it marks the
/// job running so concurrent workers never exceed the limit; returns none
/// when the job has no limit, or was cancelled or ended while it waited.
pub(super) async fn wait_for_place(
    store: &DispatchStore,
    grants: &OpenAgentStore,
    job_id: &str,
) -> Result<Option<WorkspaceLock>> {
    let job = store.load_job(job_id)?;
    let (Some(connection_id), Some(delegation)) =
        (&job.request.source_connection_id, &job.request.delegation)
    else {
        return Ok(None);
    };
    let mut since_ms = store.read_hold(job_id).map(|hold| hold.since_ms);
    loop {
        let state = store.load_state(job_id)?;
        if state.state.is_terminal() || state.cancel_requested() {
            store.clear_hold(job_id);
            return Ok(None);
        }
        // A task waiting for a grant the owner withdrew never starts.
        if let Ok(Err(_)) = grants
            .authorize_connection(connection_id, &delegation.open_agent_id)
            .await
        {
            store.clear_hold(job_id);
            bail!(GRANT_WITHDRAWN);
        }
        let limits = limits_for(grants, connection_id, &delegation.open_agent_id).await?;
        let Some(max) = limits.max_concurrent_jobs else {
            store.clear_hold(job_id);
            return Ok(None);
        };
        let admission = store.lock_connection_admission(connection_id)?;
        let waited_since = since_ms.unwrap_or_else(now_ms);
        if has_place(
            store,
            job_id,
            waited_since,
            connection_id,
            &delegation.open_agent_id,
            max,
        )? {
            store.clear_hold(job_id);
            return Ok(Some(admission));
        }
        if since_ms.is_none() {
            tracing::info!(
                "Delegated job waits for a place on its connection: job_id={}, limit={}",
                job_id,
                max
            );
        }
        since_ms = Some(waited_since);
        store.write_hold(
            job_id,
            &DispatchHold {
                reason: DispatchHoldReason::ConnectionLimit,
                limit: Some(max),
                since_ms: waited_since,
            },
        )?;
        drop(admission);
        tokio::time::sleep(PLACE_INTERVAL).await;
    }
}

/// Watches one turn of a cross-account job.
pub(super) struct ConnectionGuard {
    grants: OpenAgentStore,
    connections: ConnectionStore,
    connection_id: String,
    open_agent_id: String,
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
            .authorize_connection(&self.connection_id, &self.open_agent_id)
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

    fn request(job_id: &str, connection_id: &str) -> DispatchSubmitRequest {
        DispatchSubmitRequest {
            protocol_version: DISPATCH_PROTOCOL_VERSION,
            job_id: job_id.to_string(),
            session_id: format!("session-{job_id}"),
            workspace_path: "/tmp".to_string(),
            agent_type: "assistant:workspace".to_string(),
            prompt: "hello".to_string(),
            approval_policy: DispatchApprovalPolicy::Remote,
            model: None,
            reasoning_preset: None,
            title: None,
            attachments: Vec::new(),
            setup_audit: Vec::new(),
            delegation: Some(DispatchDelegation {
                open_agent_id: "helper".to_string(),
                source: DispatchDelegationSource::default(),
            }),
            source_device_id: None,
            source_connection_id: Some(connection_id.to_string()),
            assistant_workspace: None,
        }
    }

    async fn grants(dir: &std::path::Path, limits: ConnectionLimits) -> OpenAgentStore {
        let grants = OpenAgentStore::new(dir.join("open-agents.json"));
        crate::dispatch::delegation::test_support::open_assistant(
            &grants,
            &dir.join("personal_assistant"),
            "helper",
            "workspace",
            &[],
        )
        .await;
        grants.grant_connection("helper", "conn-1").await.unwrap();
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

    fn create(store: &DispatchStore, job_id: &str, connection_id: &str) {
        let job = request(job_id, connection_id);
        store
            .create_job_with_intent(job.clone(), job, job_id.to_string())
            .unwrap();
    }

    #[tokio::test]
    async fn tasks_over_the_limit_wait_in_order_and_start_when_a_place_frees() {
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
        for job_id in ["delegation-1", "delegation-2", "delegation-3"] {
            create(&store, job_id, "conn-1");
        }
        // Another connection, granted without a limit, keeps its own count.
        grants.grant_connection("helper", "conn-2").await.unwrap();
        create(&store, "delegation-other", "conn-2");

        // The first task takes the only place.
        let place = wait_for_place(&store, &grants, "delegation-1")
            .await
            .unwrap();
        assert!(place.is_some());
        store
            .mark_state("delegation-1", DispatchJobState::Running, None, None)
            .unwrap();
        drop(place);
        assert!(wait_for_place(&store, &grants, "delegation-other")
            .await
            .unwrap()
            .is_none());

        // The second waits, queued, and says why.
        let waiting = tokio::time::timeout(
            Duration::from_millis(200),
            wait_for_place(&store, &grants, "delegation-2"),
        )
        .await;
        assert!(waiting.is_err(), "the second task started over the limit");
        let hold = store
            .read_hold("delegation-2")
            .expect("a waiting task says why");
        assert_eq!(hold.reason, DispatchHoldReason::ConnectionLimit);
        assert_eq!(hold.limit, Some(1));

        // When the first ends, the task that waited longer goes first.
        store
            .mark_state("delegation-1", DispatchJobState::Succeeded, None, None)
            .unwrap();
        assert!(!has_place(
            &store,
            "delegation-3",
            hold.since_ms + 1,
            "conn-1",
            "helper",
            1
        )
        .unwrap());
        let place = wait_for_place(&store, &grants, "delegation-2")
            .await
            .unwrap();
        assert!(place.is_some());
        assert_eq!(store.read_hold("delegation-2"), None);
    }

    #[tokio::test]
    async fn a_waiting_task_stops_when_cancelled_or_its_grant_is_withdrawn() {
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
        for job_id in ["delegation-1", "delegation-2", "delegation-3"] {
            create(&store, job_id, "conn-1");
        }
        store
            .mark_state("delegation-1", DispatchJobState::Running, None, None)
            .unwrap();

        store.request_cancel("delegation-2").unwrap();
        assert!(wait_for_place(&store, &grants, "delegation-2")
            .await
            .unwrap()
            .is_none());

        grants.revoke_connection("helper", "conn-1").await.unwrap();
        let Err(error) = wait_for_place(&store, &grants, "delegation-3").await else {
            panic!("a task waited for a withdrawn grant");
        };
        assert_eq!(error.to_string(), GRANT_WITHDRAWN);
        assert_eq!(store.read_hold("delegation-3"), None);
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
        let guard = ConnectionGuard::for_job(
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

        let revoked = ConnectionGuard::for_job(
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
