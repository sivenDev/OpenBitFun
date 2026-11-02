use super::*;
use openbitfun_agent_runtime::permission::PermissionRequestManager;
use openbitfun_runtime_ports::{
    PermissionReply, PortError, PortResult, RemoteAgentDescriptor, RemoteAgentListing,
};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Mutex;

/// Device whose job answers each observation with the next scripted page.
#[derive(Default)]
struct ScriptedDevice {
    pages: Mutex<VecDeque<PortResult<RemoteAgentJobSnapshot>>>,
    observed_cursors: Mutex<Vec<u64>>,
    cancelled: Mutex<Vec<String>>,
    answers: Mutex<Vec<(String, PermissionReply)>>,
    cancel_seen: tokio::sync::Notify,
    answer_seen: tokio::sync::Notify,
}

impl ScriptedDevice {
    fn with_pages(pages: Vec<PortResult<RemoteAgentJobSnapshot>>) -> Arc<Self> {
        Arc::new(Self {
            pages: Mutex::new(pages.into()),
            ..Self::default()
        })
    }
}

#[async_trait::async_trait]
impl RemoteAgentDelegationPort for ScriptedDevice {
    async fn list_agents(&self, _listing: RemoteAgentListing) -> Vec<RemoteAgentDescriptor> {
        Vec::new()
    }

    async fn submit(&self, _request: RemoteAgentSubmitRequest) -> PortResult<RemoteAgentJob> {
        unreachable!("observation tests never submit")
    }

    async fn observe(
        &self,
        _job: &RemoteAgentJob,
        cursor: u64,
    ) -> PortResult<RemoteAgentJobSnapshot> {
        self.observed_cursors.lock().unwrap().push(cursor);
        let next = self.pages.lock().unwrap().pop_front();
        match next {
            Some(page) => page,
            // Keep the job running until the test decides otherwise.
            None => Ok(page(RemoteAgentJobState::Running, cursor, Vec::new())),
        }
    }

    async fn cancel(&self, job: &RemoteAgentJob) -> PortResult<bool> {
        self.cancelled.lock().unwrap().push(job.job_id.clone());
        self.cancel_seen.notify_one();
        Ok(true)
    }

    async fn answer(
        &self,
        _job: &RemoteAgentJob,
        request_id: &str,
        reply: PermissionReply,
    ) -> PortResult<()> {
        self.answers
            .lock()
            .unwrap()
            .push((request_id.to_string(), reply));
        self.answer_seen.notify_one();
        Ok(())
    }

    async fn continue_job(
        &self,
        _job: &RemoteAgentJob,
        _turn_id: &str,
        _prompt: String,
    ) -> PortResult<()> {
        Ok(())
    }
}

fn page(state: RemoteAgentJobState, cursor: u64, events: Vec<Value>) -> RemoteAgentJobSnapshot {
    RemoteAgentJobSnapshot {
        state,
        cursor,
        events,
        pending_permissions: Vec::new(),
        cursor_reset: false,
        last_error: None,
        hold: None,
    }
}

/// A raw dispatch event as the target CLI writes it.
fn agent_event(event: Value) -> Value {
    json!({
        "type": "agentEvent",
        "timestamp": "2026-10-30T00:00:00Z",
        "event": { "id": "envelope", "event": event, "priority": "Normal", "timestamp": { "secs_since_epoch": 0, "nanos_since_epoch": 0 } },
    })
}

fn round(session: &str, round_id: &str, index: usize) -> Value {
    agent_event(json!({
        "type": "ModelRoundStarted", "session_id": session, "turn_id": "t",
        "round_id": round_id, "round_index": index,
        "model_config_id": "m", "effective_model_name": "m",
    }))
}

fn text(session: &str, round_id: &str, value: &str) -> Value {
    agent_event(json!({
        "type": "TextChunk", "session_id": session, "turn_id": "t",
        "round_id": round_id, "text": value,
    }))
}

fn link() -> RemoteJobLink {
    RemoteJobLink {
        job: RemoteAgentJob {
            device_id: "laptop".to_string(),
            job_id: "delegation-1".to_string(),
            session_id: "job-session".to_string(),
        },
        cursor: 4,
    }
}

#[test]
fn final_text_is_the_last_round_with_text_of_the_job_session() {
    let mut text_state = FinalText::default();
    let progress = text_state.absorb(
        &[
            json!({"type": "jobState", "state": "running"}),
            round("job-session", "r1", 0),
            text("job-session", "r1", "Let me look."),
            round("job-session", "r2", 1),
            text("nested-subagent", "x", "not the answer"),
            agent_event(json!({"type": "SomethingNewer", "session_id": "job-session"})),
            round("job-session", "r3", 2),
            text("job-session", "r3", "The answer "),
            text("job-session", "r3", "is 42."),
        ],
        "job-session",
    );
    assert_eq!(text_state.finish(), "The answer is 42.");
    // Only the job session's rounds and text are projected.
    assert_eq!(progress.len(), 6, "{progress:?}");

    // A round without text keeps the earlier answer.
    text_state.absorb(&[round("job-session", "r4", 3)], "job-session");
    assert_eq!(text_state.finish(), "The answer is 42.");
}

#[test]
fn job_links_round_trip_and_reject_unreadable_values() {
    let encoded = link().encode();
    let value: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["deviceId"], "laptop");
    assert_eq!(value["cursor"], 4);
    assert_eq!(RemoteJobLink::parse(&encoded), Some(link()));
    assert_eq!(RemoteJobLink::parse("not json"), None);
}

#[tokio::test(start_paused = true)]
async fn observation_follows_the_cursor_and_retries_unreachable_devices() {
    let device = ScriptedDevice::with_pages(vec![
        Ok(page(
            RemoteAgentJobState::Running,
            6,
            vec![
                round("job-session", "r1", 0),
                text("job-session", "r1", "Done."),
            ],
        )),
        Err(PortError::new(PortErrorKind::Backend, "device offline")),
        Ok(page(RemoteAgentJobState::Succeeded, 7, Vec::new())),
    ]);
    let port: Arc<dyn RemoteAgentDelegationPort> = device.clone();
    let (end, cursor) =
        observe_job(&port, &link(), &CancellationToken::new(), &mut ResumedNoop).await;
    assert_eq!(
        end,
        RemoteJobEnd::Succeeded {
            final_text: "Done.".to_string()
        }
    );
    assert_eq!(cursor, 7);
    assert_eq!(*device.observed_cursors.lock().unwrap(), vec![4, 6, 6]);
}

#[tokio::test(start_paused = true)]
async fn failed_jobs_report_the_target_error() {
    let mut failed = page(RemoteAgentJobState::Failed, 5, Vec::new());
    failed.last_error = Some("open agent 'helper' is closed".to_string());
    let port: Arc<dyn RemoteAgentDelegationPort> = ScriptedDevice::with_pages(vec![Ok(failed)]);
    let (end, _) = observe_job(&port, &link(), &CancellationToken::new(), &mut ResumedNoop).await;
    assert_eq!(
        end,
        RemoteJobEnd::Failed {
            error: "open agent 'helper' is closed".to_string()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn cancelling_the_observation_cancels_the_job_on_its_device() {
    let device = ScriptedDevice::with_pages(Vec::new());
    let port: Arc<dyn RemoteAgentDelegationPort> = device.clone();
    let cancel = CancellationToken::new();
    let observation = {
        let cancel = cancel.clone();
        let port = port.clone();
        tokio::spawn(async move { observe_job(&port, &link(), &cancel, &mut ResumedNoop).await })
    };
    tokio::time::sleep(Duration::from_secs(5)).await;
    cancel.cancel();
    let (end, _) = observation.await.unwrap();
    assert!(matches!(end, RemoteJobEnd::Cancelled { .. }));
    device.cancel_seen.notified().await;
    assert_eq!(*device.cancelled.lock().unwrap(), vec!["delegation-1"]);
}

struct ResumedNoop;

#[async_trait::async_trait]
impl JobPageSink for ResumedNoop {
    async fn on_page(&mut self, _: &RemoteAgentJobSnapshot, _: Vec<ExternalAgentTurnEvent>) {}
}

fn test_permission_manager() -> Arc<PermissionRequestManager> {
    let root = std::env::temp_dir().join(format!(
        "openbitfun-remote-agent-permission-test-{}",
        uuid::Uuid::new_v4()
    ));
    let store = Arc::new(
        openbitfun_services_core::permission_store::ProjectPermissionSqliteStore::new(
            root.join("permissions"),
        ),
    );
    Arc::new(PermissionRequestManager::new(
        store.clone(),
        store,
        Arc::new(
            openbitfun_runtime_services::test_support::FakeRuntimePort::new(
                openbitfun_runtime_ports::RuntimeServiceCapability::Clock,
            ),
        ),
    ))
}

fn target_prompt(request_id: &str) -> PermissionRequest {
    PermissionRequest {
        request_id: request_id.to_string(),
        round_id: "target-round".to_string(),
        order: 0,
        tool_call_id: Some("target-tool".to_string()),
        project_path: Some("/target/workspace".to_string()),
        project_id: "target-project".to_string(),
        session_id: "job-session".to_string(),
        agent_id: "Standard".to_string(),
        action: "bash".to_string(),
        resources: vec!["rm -rf build".to_string()],
        save_resources: vec!["rm *".to_string()],
        source: PermissionRequestSource {
            kind: PermissionRequestSourceKind::Extension,
            identity: "Standard".to_string(),
        },
        delegation: None,
        display_metadata: Default::default(),
    }
}

#[tokio::test]
async fn target_prompts_are_asked_here_and_answered_on_the_device() {
    let manager = test_permission_manager();
    let device = ScriptedDevice::with_pages(Vec::new());
    let port: Arc<dyn RemoteAgentDelegationPort> = device.clone();
    let cancel = CancellationToken::new();
    let mut forwarder = PromptForwarder {
        route: Some(ForwardRoute {
            manager: manager.clone(),
            project_id: "local-project".to_string(),
            project_path: "/local/workspace".to_string(),
            delegation: None,
            session_id: "child".to_string(),
            turn_id: "child-turn".to_string(),
            agent_type: "remote:laptop/helper".to_string(),
            device_id: "laptop".to_string(),
            device_name: "Laptop".to_string(),
            job: link().job,
            port,
            cancel: cancel.clone(),
        }),
        forwarded: HashMap::new(),
        owner_wait: None,
    };
    // A job of this account waits on nobody else.
    assert!(forwarder
        .owner_wait_note(&[target_prompt("target-1")])
        .is_none());

    forwarder
        .sync(&[target_prompt("target-1")], "child-round".to_string())
        .await;
    // Seeing the same prompt again files nothing new.
    forwarder
        .sync(&[target_prompt("target-1")], "child-round".to_string())
        .await;
    let pending = manager.pending_requests();
    assert_eq!(pending.len(), 1);
    let local = &pending[0];
    assert_eq!(local.session_id, "child");
    assert_eq!(local.round_id, "child-round");
    assert_eq!(local.project_id, "local-project");
    assert!(
        local.save_resources.is_empty(),
        "a target resource never becomes a local grant"
    );
    assert_eq!(local.display_metadata["remoteDeviceId"], "laptop");
    assert_eq!(local.display_metadata["remoteDeviceName"], "Laptop");

    manager
        .reply(
            &local.request_id,
            PermissionReply::Once,
            openbitfun_runtime_ports::PermissionReplySource::User,
        )
        .await
        .unwrap();
    device.answer_seen.notified().await;
    assert_eq!(
        *device.answers.lock().unwrap(),
        vec![("target-1".to_string(), PermissionReply::Once)]
    );

    // A prompt answered on the device itself is withdrawn here.
    forwarder
        .sync(&[target_prompt("target-2")], "child-round".to_string())
        .await;
    assert_eq!(manager.pending_requests().len(), 1);
    forwarder.sync(&[], String::new()).await;
    assert!(manager.pending_requests().is_empty());
    cancel.cancel();
}

#[tokio::test]
async fn prompts_of_another_accounts_job_wait_for_its_owner_and_are_never_filed_here() {
    let manager = test_permission_manager();
    let device = ScriptedDevice::with_pages(Vec::new());
    let port: Arc<dyn RemoteAgentDelegationPort> = device.clone();
    let cancel = CancellationToken::new();
    let mut job = link().job;
    job.device_id = openbitfun_runtime_ports::remote_connection_target("c1");
    let mut forwarder = PromptForwarder::new(
        None,
        PromptScope {
            session_id: "child",
            turn_id: "child-turn",
            agent_type: "remote:conn:c1/helper",
            cancel: &cancel,
        },
        &job,
        &port,
        "B1 Desktop".to_string(),
    );

    let note = forwarder.owner_wait_note(&[target_prompt("target-1")]);
    assert!(
        matches!(&note, Some(ExternalAgentTurnEvent::Thought { text }) if *text == owner_approval_note("B1 Desktop")),
        "{note:?}"
    );
    // The same prompt is noted once; a new one is noted again.
    assert!(forwarder
        .owner_wait_note(&[target_prompt("target-1")])
        .is_none());
    assert!(forwarder
        .owner_wait_note(&[target_prompt("target-1"), target_prompt("target-2")])
        .is_some());

    forwarder
        .sync(&[target_prompt("target-1")], "child-round".to_string())
        .await;
    assert!(manager.pending_requests().is_empty());
    assert!(device.answers.lock().unwrap().is_empty());
    cancel.cancel();
}

#[tokio::test]
async fn a_task_resumed_after_restart_forwards_the_prompt_its_device_still_waits_on() {
    let root = tempfile::tempdir().expect("temp dir");
    let store = Arc::new(CoordinationStore::new(
        root.path().join("coordination.sqlite"),
    ));
    let task = store
        .register_background_task(
            super::super::coordination_store::BackgroundTaskRegistration {
                parent_session_id: "parent".to_string(),
                requested_agent_id: None,
                child_session_id: "child".to_string(),
                parent_dialog_turn_id: "parent-turn".to_string(),
                parent_tool_call_id: "spawn".to_string(),
                child_dialog_turn_id: "child-turn".to_string(),
            },
        )
        .await
        .expect("register delegated task");
    let manager = test_permission_manager();
    let mut waiting = page(RemoteAgentJobState::Running, 3, Vec::new());
    waiting.pending_permissions = vec![target_prompt("target-1")];
    let device = ScriptedDevice::with_pages(vec![Ok(waiting.clone()), Ok(waiting)]);
    let port: Arc<dyn RemoteAgentDelegationPort> = device.clone();
    let route = ExternalAgentPermissionRoute {
        manager: manager.clone(),
        project_id: "local-project".to_string(),
        project_path: "/local/workspace".to_string(),
        delegation: None,
        policy: None,
    };
    let cancel = CancellationToken::new();
    let mut sink = ResumedTaskSink::new(
        ResumedTask {
            store: Arc::clone(&store),
            event_queue: Arc::new(crate::agentic::events::EventQueue::new(
                crate::agentic::events::EventQueueConfig::default(),
            )),
            task_pk: task.task_pk,
            session_id: "child",
            turn_id: "child-turn",
            agent_type: "remote:laptop/helper",
            permissions: Some(&route),
        },
        &link(),
        &port,
        cancel.clone(),
    )
    .await;

    let observation = tokio::spawn(async move {
        let end = observe_job(&port, &link(), &cancel, &mut sink).await;
        sink.finish().await;
        end
    });
    let local = loop {
        if let Some(request) = manager.pending_requests().pop() {
            break request;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(local.session_id, "child");
    assert_eq!(local.agent_id, "remote:laptop/helper");
    assert_eq!(local.display_metadata["remoteDeviceId"], "laptop");
    manager
        .reply(
            &local.request_id,
            PermissionReply::Once,
            openbitfun_runtime_ports::PermissionReplySource::User,
        )
        .await
        .unwrap();
    device.answer_seen.notified().await;
    assert_eq!(
        *device.answers.lock().unwrap(),
        vec![("target-1".to_string(), PermissionReply::Once)]
    );

    device
        .pages
        .lock()
        .unwrap()
        .push_back(Ok(page(RemoteAgentJobState::Succeeded, 4, Vec::new())));
    let (end, _) = observation.await.expect("observation finishes");
    assert!(matches!(end, RemoteJobEnd::Succeeded { .. }));
    assert!(manager.pending_requests().is_empty());
}

#[test]
fn the_parent_hears_why_a_job_waits_on_its_device() {
    let root = tempfile::tempdir().expect("temp dir");
    let store = Arc::new(CoordinationStore::new(
        root.path().join("coordination.sqlite"),
    ));
    let mut other_account = link().job;
    other_account.device_id = openbitfun_runtime_ports::remote_connection_target("c1");
    let mut holds = HoldTracker::new(store.clone(), "child", &other_account, "B1".to_string());

    // Queued on the device: the child session says so once, the parent
    // learns the limit and that it starts on its own.
    let mut queued = page(RemoteAgentJobState::Queued, 0, Vec::new());
    queued.hold = Some(RemoteAgentJobHold {
        reason: openbitfun_runtime_ports::RemoteAgentHoldReason::ConnectionLimit,
        limit: Some(1),
    });
    let note = holds.observe(&queued);
    assert!(
        matches!(&note, Some(ExternalAgentTurnEvent::Thought { text }) if *text == queued_note("B1")),
        "{note:?}"
    );
    assert!(holds.observe(&queued).is_none());
    let reason = store.remote_holds().note("child").expect("queued hold");
    assert!(reason.contains("allows 1 task(s) at a time"), "{reason}");
    assert!(reason.contains("Unblock:"), "{reason}");

    // Running and asking the other account's owner: the parent learns who
    // must act.
    let mut asking = page(RemoteAgentJobState::Running, 1, Vec::new());
    asking.pending_permissions = vec![target_prompt("target-1")];
    assert!(holds.observe(&asking).is_none());
    let reason = store.remote_holds().note("child").expect("owner hold");
    assert!(reason.contains("Unblock: the owner of B1"), "{reason}");

    // Answered: nothing holds it any more.
    holds.observe(&page(RemoteAgentJobState::Running, 2, Vec::new()));
    assert_eq!(store.remote_holds().note("child"), None);

    // A job of this account asks this host, so its prompts hold nothing.
    let mut own = HoldTracker::new(store.clone(), "own", &link().job, "A2".to_string());
    own.observe(&asking);
    assert_eq!(store.remote_holds().note("own"), None);
    holds.observe(&queued);
    holds.finish();
    assert_eq!(store.remote_holds().note("child"), None);
}
