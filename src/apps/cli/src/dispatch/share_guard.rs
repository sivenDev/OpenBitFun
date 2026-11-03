//! How many tasks a shared assistant runs at once for other devices, and the
//! guard that stops a task when its owner withdraws access.
//!
//! The owner sets the number of concurrent tasks on the share; it counts the
//! tasks of every device the assistant is shared with. A task over it is
//! accepted and waits, queued, until one of them ends; waiting tasks start in
//! the order they began to wait. The grant is checked while a turn runs, so
//! revoking a device, blocking it or stopping the share stops work already in
//! progress.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use openbitfun_core::service::lan_share::SharedAssistantStore;

use super::protocol::{DispatchHold, DispatchHoldReason, DispatchJobState, DispatchSubmitRequest};
use super::store::{DispatchJobRecord, DispatchStore, WorkspaceLock};

/// How often a running task from another device checks its guard.
pub(super) const GUARD_INTERVAL: Duration = Duration::from_secs(1);
/// How often a queued task looks for a free place.
const PLACE_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_secs(2)
};
/// Why a task from another device stopped, as its requester reads it.
pub(super) const GRANT_WITHDRAWN: &str =
    "The owner of the target device withdrew this device's access to the assistant";

/// The shared assistant a job from another device runs, with that device's
/// sharing key.
fn share_of(request: &DispatchSubmitRequest) -> Option<(&str, &str)> {
    Some((
        request.delegation.as_ref()?.assistant_id.as_str(),
        request.source_share_key.as_deref()?,
    ))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// Whether `job_id`, waiting since `since_ms`, may start now: the tasks the
/// shared assistant runs for other devices, and those that began to wait
/// before it, leave a place under `max`.
fn has_place(
    store: &DispatchStore,
    job_id: &str,
    since_ms: u64,
    assistant_id: &str,
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
        if share_of(&record.request).is_none_or(|(other, _)| other != assistant_id) {
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

/// Waits until the task `job_id` from another device may run under its
/// share's limit, recording why it waits meanwhile. Returns the share's
/// admission lock, which the caller holds until it marks the job running so
/// concurrent workers never exceed the limit; returns none for jobs of this
/// device, or when the job was cancelled or ended while it waited.
pub(super) async fn wait_for_place(
    store: &DispatchStore,
    grants: &SharedAssistantStore,
    job_id: &str,
) -> Result<Option<WorkspaceLock>> {
    let job = store.load_job(job_id)?;
    let Some((assistant_id, share_key)) = share_of(&job.request) else {
        return Ok(None);
    };
    let mut since_ms = store.read_hold(job_id).map(|hold| hold.since_ms);
    loop {
        let state = store.load_state(job_id)?;
        if state.state.is_terminal() || state.cancel_requested() {
            store.clear_hold(job_id);
            return Ok(None);
        }
        // A task waiting for access the owner withdrew never starts.
        let share = match grants
            .authorize(assistant_id, share_key)
            .await
            .context("read shared assistants")?
        {
            Ok((share, _)) => share,
            Err(_) => {
                store.clear_hold(job_id);
                bail!(GRANT_WITHDRAWN);
            }
        };
        let max = share.settings.max_concurrent_tasks;
        let admission = store.lock_share_admission(assistant_id)?;
        let waited_since = since_ms.unwrap_or_else(now_ms);
        if has_place(store, job_id, waited_since, assistant_id, max)? {
            store.clear_hold(job_id);
            return Ok(Some(admission));
        }
        if since_ms.is_none() {
            tracing::info!(
                "Task for a shared assistant waits for a place: job_id={}, limit={}",
                job_id,
                max
            );
        }
        since_ms = Some(waited_since);
        store.write_hold(
            job_id,
            &DispatchHold {
                reason: DispatchHoldReason::ShareLimit,
                limit: Some(max),
                since_ms: waited_since,
            },
        )?;
        drop(admission);
        tokio::time::sleep(PLACE_INTERVAL).await;
    }
}

/// Watches one turn of a task from another device.
pub(super) struct ShareGuard {
    grants: SharedAssistantStore,
    assistant_id: String,
    share_key: String,
}

impl ShareGuard {
    /// The guard for `job`, or none for a job of this device.
    pub(super) fn for_job(job: &DispatchJobRecord, grants: SharedAssistantStore) -> Option<Self> {
        let (assistant_id, share_key) = share_of(&job.request)?;
        Some(Self {
            grants,
            assistant_id: assistant_id.to_string(),
            share_key: share_key.to_string(),
        })
    }

    /// Why the turn must stop now, if it must.
    pub(super) async fn stop_reason(&self) -> Option<String> {
        match self
            .grants
            .authorize(&self.assistant_id, &self.share_key)
            .await
        {
            Ok(Ok(_)) => None,
            Ok(Err(_)) => Some(GRANT_WITHDRAWN.to_string()),
            // An unreadable list is not a withdrawal; the next check retries.
            Err(error) => {
                tracing::warn!("Shared assistant grant check unavailable: {error}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::delegation::test_support::{peer, share_assistant, share_key};
    use crate::dispatch::protocol::{
        DispatchApprovalPolicy, DispatchDelegation, DispatchDelegationSource,
        DISPATCH_PROTOCOL_VERSION,
    };
    use openbitfun_core::service::lan_share::ShareSettings;

    fn request(job_id: &str, assistant_id: &str, seed: Option<u8>) -> DispatchSubmitRequest {
        DispatchSubmitRequest {
            protocol_version: DISPATCH_PROTOCOL_VERSION,
            job_id: job_id.to_string(),
            session_id: format!("session-{job_id}"),
            workspace_path: "/tmp".to_string(),
            agent_type: format!("assistant:{assistant_id}"),
            prompt: "hello".to_string(),
            approval_policy: DispatchApprovalPolicy::Remote,
            model: None,
            reasoning_preset: None,
            title: None,
            attachments: Vec::new(),
            setup_audit: Vec::new(),
            delegation: Some(DispatchDelegation {
                assistant_id: assistant_id.to_string(),
                source: DispatchDelegationSource::default(),
                call_chain: Vec::new(),
            }),
            source_share_key: seed.map(share_key),
            source_user: None,
            assistant_workspace: None,
        }
    }

    /// `workspace` shared with devices 1 and 2 at one task at a time, and
    /// `workspace-notes` with device 1 at the default limit.
    async fn grants(dir: &std::path::Path) -> SharedAssistantStore {
        let grants = SharedAssistantStore::new(dir.join("shared-assistants.json"));
        let assistants = dir.join("personal_assistant");
        let one_at_a_time = ShareSettings {
            max_concurrent_tasks: 1,
            ..ShareSettings::default()
        };
        share_assistant(&grants, &assistants, "workspace", one_at_a_time, &[1, 2]).await;
        share_assistant(
            &grants,
            &assistants,
            "workspace-notes",
            ShareSettings::default(),
            &[1],
        )
        .await;
        grants
    }

    fn create(store: &DispatchStore, job_id: &str, assistant_id: &str, seed: u8) {
        let job = request(job_id, assistant_id, Some(seed));
        store
            .create_job_with_intent(job.clone(), job, job_id.to_string())
            .unwrap();
    }

    #[tokio::test]
    async fn tasks_over_the_share_limit_wait_in_order_across_devices() {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
        let grants = grants(dir.path()).await;
        create(&store, "delegation-1", "workspace", 1);
        // Another device of the same share counts against the same limit.
        create(&store, "delegation-2", "workspace", 2);
        create(&store, "delegation-3", "workspace", 1);
        // Another shared assistant keeps its own count.
        create(&store, "delegation-other", "workspace-notes", 1);

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
            .is_some());

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
        assert_eq!(hold.reason, DispatchHoldReason::ShareLimit);
        assert_eq!(hold.limit, Some(1));

        // When the first ends, the task that waited longer goes first.
        store
            .mark_state("delegation-1", DispatchJobState::Succeeded, None, None)
            .unwrap();
        assert!(!has_place(&store, "delegation-3", hold.since_ms + 1, "workspace", 1).unwrap());
        let place = wait_for_place(&store, &grants, "delegation-2")
            .await
            .unwrap();
        assert!(place.is_some());
        assert_eq!(store.read_hold("delegation-2"), None);
    }

    #[tokio::test]
    async fn a_waiting_task_stops_when_cancelled_or_its_access_is_withdrawn() {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
        let grants = grants(dir.path()).await;
        for job_id in ["delegation-1", "delegation-2", "delegation-3"] {
            create(&store, job_id, "workspace", 1);
        }
        store
            .mark_state("delegation-1", DispatchJobState::Running, None, None)
            .unwrap();

        store.request_cancel("delegation-2").unwrap();
        assert!(wait_for_place(&store, &grants, "delegation-2")
            .await
            .unwrap()
            .is_none());

        grants.revoke("workspace", &share_key(1)).await.unwrap();
        let Err(error) = wait_for_place(&store, &grants, "delegation-3").await else {
            panic!("a task waited for withdrawn access");
        };
        assert_eq!(error.to_string(), GRANT_WITHDRAWN);
        assert_eq!(store.read_hold("delegation-3"), None);
    }

    #[tokio::test]
    async fn running_turns_stop_when_access_ends() {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
        let grants = grants(dir.path()).await;
        create(&store, "delegation-1", "workspace", 1);
        create(&store, "delegation-2", "workspace", 2);
        let guard_of = |job_id: &str| {
            ShareGuard::for_job(&store.load_job(job_id).unwrap(), grants.clone())
                .expect("a task from another device is guarded")
        };
        let first = guard_of("delegation-1");
        let second = guard_of("delegation-2");
        assert_eq!(first.stop_reason().await, None);

        // Blocking one device stops its turns only.
        grants.block("workspace", peer(1)).await.unwrap();
        assert_eq!(first.stop_reason().await.as_deref(), Some(GRANT_WITHDRAWN));
        assert_eq!(second.stop_reason().await, None);
        // Stopping the share stops every device's.
        grants.stop("workspace").await.unwrap();
        assert_eq!(second.stop_reason().await.as_deref(), Some(GRANT_WITHDRAWN));

        // Jobs of this device are not guarded.
        let local = request("dispatch-local", "workspace", None);
        store
            .create_job_with_intent(local.clone(), local, "local".to_string())
            .unwrap();
        assert!(ShareGuard::for_job(&store.load_job("dispatch-local").unwrap(), grants).is_none());
    }
}
