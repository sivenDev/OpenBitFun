//! Delegated subagent turns that run on another device of this account.
//!
//! A subagent whose agent type is `remote:<deviceId>/<openAgentId>` keeps the
//! native background subagent lifecycle (session, cancellation, AgentWait);
//! only the turn body is replaced. The turn submits a detached dispatch job
//! to the device (or continues the session's earlier job), observes its event
//! log by cursor, projects the job session's progress onto the child session,
//! and forwards the job's permission prompts to whoever drives this session.
//! Prompts are always asked: they concern the other device's resources, so no
//! local rule or auto-approval decides them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{debug, warn};
use openbitfun_agent_runtime::permission::PermissionWaitOutcome;
use openbitfun_runtime_ports::{
    ExternalAgentTurnEvent, PermissionRequest, PermissionRequestSource,
    PermissionRequestSourceKind, PortErrorKind, RemoteAgentAddress, RemoteAgentDelegationPort,
    RemoteAgentJob, RemoteAgentJobHold, RemoteAgentJobSnapshot, RemoteAgentJobState,
    RemoteAgentSource, RemoteAgentSubmitRequest,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::coordination_store::CoordinationStore;
use super::external_agent_turn::{
    execution_result, ChildSessionStream, ExternalAgentPermissionRoute,
};
use crate::agentic::events::{AgenticEvent, EventPriority, EventQueue};
use crate::agentic::execution::{ExecutionResult, FinishReason};
use crate::util::errors::{OpenBitFunError, OpenBitFunResult};

/// Delay between observations of a running job. A full page is read again
/// at once.
const OBSERVE_INTERVAL: Duration = Duration::from_millis(1500);
/// Upper bound on the delay between retries while the device is unreachable.
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) fn remote_agent_address(agent_type: &str) -> Option<RemoteAgentAddress> {
    RemoteAgentAddress::parse(agent_type)
}

/// The delegated job of a background task, persisted with the task so a
/// follow-up turn continues it and a restarted host observes it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteJobLink {
    #[serde(flatten)]
    pub(crate) job: RemoteAgentJob,
    /// Event cursor where the task's own turn begins on the target.
    pub(crate) cursor: u64,
}

impl RemoteJobLink {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        serde_json::from_str(value).ok()
    }

    fn encode(&self) -> String {
        serde_json::to_string(self).expect("remote job link serializes")
    }
}

pub(crate) struct RemoteAgentTurn {
    pub(crate) port: Option<Arc<dyn RemoteAgentDelegationPort>>,
    pub(crate) coordination_store: Arc<CoordinationStore>,
    pub(crate) event_queue: Arc<EventQueue>,
    pub(crate) agent_type: String,
    pub(crate) parent_session_id: String,
    pub(crate) session_id: String,
    pub(crate) dialog_turn_id: String,
    pub(crate) prompt: String,
    pub(crate) source: RemoteAgentSource,
    pub(crate) permissions: Option<ExternalAgentPermissionRoute>,
    pub(crate) cancel_token: CancellationToken,
}

/// How an observed job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteJobEnd {
    Succeeded { final_text: String },
    Failed { error: String },
    Cancelled { partial_text: String },
}

/// Runs one delegated turn and shapes its result like a native dialog turn so
/// the coordinator's completion and cancellation paths apply as-is.
pub(crate) async fn run_remote_agent_turn(
    turn: RemoteAgentTurn,
) -> OpenBitFunResult<ExecutionResult> {
    let started_at = Instant::now();
    let agent_type = turn.agent_type.clone();
    let address = remote_agent_address(&agent_type).ok_or_else(|| {
        OpenBitFunError::Validation(format!("'{agent_type}' is not a remote agent type"))
    })?;
    let port = turn.port.clone().ok_or_else(|| {
        OpenBitFunError::service(format!(
            "Remote agent '{agent_type}' is unavailable: this host cannot delegate to other devices"
        ))
    })?;

    let previous = turn
        .coordination_store
        .latest_remote_job_for_child(&turn.session_id)
        .await?
        .and_then(|value| RemoteJobLink::parse(&value));
    let link = match previous {
        Some(previous) => {
            port.continue_job(&previous.job, &turn.dialog_turn_id, turn.prompt.clone())
                .await
                .map_err(|error| remote_error(&agent_type, error))?;
            previous
        }
        None => {
            let job = port
                .submit(RemoteAgentSubmitRequest {
                    address,
                    job_id: format!(
                        "{}{}",
                        crate::service::delegation::DELEGATED_JOB_ID_PREFIX,
                        uuid::Uuid::new_v4()
                    ),
                    session_id: uuid::Uuid::new_v4().to_string(),
                    prompt: turn.prompt.clone(),
                    title: None,
                    source: turn.source.clone(),
                })
                .await
                .map_err(|error| remote_error(&agent_type, error))?;
            RemoteJobLink { job, cursor: 0 }
        }
    };
    record_link(&turn, &link).await;

    let job_device_name = device_name(&port, &link.job.device_id).await;
    let mut sink = LiveSink {
        holds: HoldTracker::new(
            turn.coordination_store.clone(),
            &turn.session_id,
            &link.job,
            job_device_name.clone(),
        ),
        stream: ChildSessionStream::new(
            &turn.session_id,
            &turn.dialog_turn_id,
            &agent_type,
            &link.job.device_id,
        ),
        event_queue: turn.event_queue.clone(),
        prompts: PromptForwarder::new(
            turn.permissions.as_ref(),
            PromptScope {
                session_id: &turn.session_id,
                turn_id: &turn.dialog_turn_id,
                agent_type: &turn.agent_type,
                cancel: &turn.cancel_token,
            },
            &link.job,
            &port,
            job_device_name,
        ),
    };
    let (end, cursor) = observe_job(&port, &link, &turn.cancel_token, &mut sink).await;
    sink.holds.finish();
    sink.prompts
        .cancel_all("The remote agent job is no longer waiting")
        .await;
    if let Some(closed) = sink.stream.close_round() {
        let _ = turn
            .event_queue
            .enqueue(closed, Some(EventPriority::Normal))
            .await;
    }
    let rounds = sink.stream.rounds;
    record_link(
        &turn,
        &RemoteJobLink {
            cursor,
            ..link.clone()
        },
    )
    .await;

    let duration_ms = started_at.elapsed().as_millis() as u64;
    match end {
        RemoteJobEnd::Succeeded { final_text } => Ok(execution_result(
            final_text,
            rounds,
            duration_ms,
            FinishReason::Complete,
        )),
        RemoteJobEnd::Cancelled { partial_text }
            if turn.cancel_token.is_cancelled() && !partial_text.trim().is_empty() =>
        {
            Ok(execution_result(
                partial_text,
                rounds,
                duration_ms,
                FinishReason::Cancelled,
            ))
        }
        RemoteJobEnd::Cancelled { .. } => Err(OpenBitFunError::Cancelled(format!(
            "Remote agent '{agent_type}' job was cancelled"
        ))),
        RemoteJobEnd::Failed { error } => Err(OpenBitFunError::service(format!(
            "Remote agent '{agent_type}' failed: {error}"
        ))),
    }
}

async fn record_link(turn: &RemoteAgentTurn, link: &RemoteJobLink) {
    match turn
        .coordination_store
        .set_remote_job(&turn.parent_session_id, &turn.dialog_turn_id, link.encode())
        .await
    {
        Ok(true) => {}
        Ok(false) => warn!(
            "Remote agent job has no background task to record it; a restart will not observe it: job_id={}",
            link.job.job_id
        ),
        Err(error) => warn!(
            "Failed to record remote agent job: job_id={}, error={}",
            link.job.job_id, error
        ),
    }
}

/// Receives each observed page with the job session's progress.
#[async_trait::async_trait]
pub(crate) trait JobPageSink: Send {
    async fn on_page(
        &mut self,
        snapshot: &RemoteAgentJobSnapshot,
        progress: Vec<ExternalAgentTurnEvent>,
    );
}

/// Observes a task this process adopted after a restart. Its progress was
/// already shown by the process that started it, so only the outcome is
/// kept, but its prompts are forwarded again: the device keeps waiting for
/// an answer the earlier process can no longer give. Cancelling the task
/// through AgentControl cancels the job too.
pub(crate) struct ResumedTaskSink {
    holds: HoldTracker,
    store: Arc<CoordinationStore>,
    task_pk: i64,
    cancel: CancellationToken,
    stream: ChildSessionStream,
    event_queue: Arc<EventQueue>,
    prompts: PromptForwarder,
}

/// The adopted task a [`ResumedTaskSink`] observes.
pub(crate) struct ResumedTask<'a> {
    pub(crate) store: Arc<CoordinationStore>,
    pub(crate) event_queue: Arc<EventQueue>,
    pub(crate) task_pk: i64,
    pub(crate) session_id: &'a str,
    pub(crate) turn_id: &'a str,
    pub(crate) agent_type: &'a str,
    pub(crate) permissions: Option<&'a ExternalAgentPermissionRoute>,
}

impl ResumedTaskSink {
    pub(crate) async fn new(
        task: ResumedTask<'_>,
        link: &RemoteJobLink,
        port: &Arc<dyn RemoteAgentDelegationPort>,
        cancel: CancellationToken,
    ) -> Self {
        let job_device_name = device_name(port, &link.job.device_id).await;
        let prompts = PromptForwarder::new(
            task.permissions,
            PromptScope {
                session_id: task.session_id,
                turn_id: task.turn_id,
                agent_type: task.agent_type,
                cancel: &cancel,
            },
            &link.job,
            port,
            job_device_name.clone(),
        );
        Self {
            holds: HoldTracker::new(
                task.store.clone(),
                task.session_id,
                &link.job,
                job_device_name,
            ),
            store: task.store,
            task_pk: task.task_pk,
            stream: ChildSessionStream::new(
                task.session_id,
                task.turn_id,
                task.agent_type,
                &link.job.device_id,
            ),
            event_queue: task.event_queue,
            cancel,
            prompts,
        }
    }

    /// Withdraws the prompts still filed once observation ends.
    pub(crate) async fn finish(mut self) {
        self.holds.finish();
        self.prompts
            .cancel_all("The remote agent job is no longer waiting")
            .await;
        if let Some(closed) = self.stream.close_round() {
            let _ = self
                .event_queue
                .enqueue(closed, Some(EventPriority::Normal))
                .await;
        }
    }
}

#[async_trait::async_trait]
impl JobPageSink for ResumedTaskSink {
    async fn on_page(&mut self, snapshot: &RemoteAgentJobSnapshot, _: Vec<ExternalAgentTurnEvent>) {
        let still_running = match self.store.records_by_task_pks(&[self.task_pk]).await {
            Ok(records) => records.iter().any(|record| {
                record.status == super::coordination_store::BackgroundTaskStatus::Running
            }),
            Err(error) => {
                debug!("Delegated task status unavailable: {error}");
                true
            }
        };
        if !still_running {
            self.holds.finish();
            self.cancel.cancel();
            return;
        }
        if let Some(note) = self.holds.observe(snapshot) {
            for event in self.stream.project(note) {
                let _ = self
                    .event_queue
                    .enqueue(event, Some(EventPriority::Normal))
                    .await;
            }
        }
        let round_id = if snapshot.pending_permissions.is_empty() {
            String::new()
        } else {
            let (opened, round_id) = self.stream.ensure_round();
            for event in opened {
                let _ = self
                    .event_queue
                    .enqueue(event, Some(EventPriority::Normal))
                    .await;
            }
            round_id
        };
        if let Some(note) = self.prompts.owner_wait_note(&snapshot.pending_permissions) {
            for event in self.stream.project(note) {
                let _ = self
                    .event_queue
                    .enqueue(event, Some(EventPriority::Normal))
                    .await;
            }
        }
        self.prompts
            .sync(&snapshot.pending_permissions, round_id)
            .await;
    }
}

/// Observes `link.job` from `link.cursor` until it ends or `cancel` fires,
/// returning how it ended and the cursor after its last event. A cancelled
/// observation asks the device to cancel without waiting for its answer.
/// An unreachable device is retried: the job keeps running there, and
/// cancellation stays available.
pub(crate) async fn observe_job(
    port: &Arc<dyn RemoteAgentDelegationPort>,
    link: &RemoteJobLink,
    cancel: &CancellationToken,
    sink: &mut dyn JobPageSink,
) -> (RemoteJobEnd, u64) {
    let mut cursor = link.cursor;
    let mut text = FinalText::default();
    let mut retry = OBSERVE_INTERVAL;
    loop {
        let observed = tokio::select! {
            observed = port.observe(&link.job, cursor), if !cancel.is_cancelled() => Some(observed),
            _ = cancel.cancelled() => None,
        };
        let Some(observed) = observed else {
            spawn_cancel(port.clone(), link.job.clone());
            return (
                RemoteJobEnd::Cancelled {
                    partial_text: text.finish(),
                },
                cursor,
            );
        };
        let wait = match observed {
            Ok(snapshot) => {
                retry = OBSERVE_INTERVAL;
                cursor = snapshot.cursor;
                let progress = text.absorb(&snapshot.events, &link.job.session_id);
                sink.on_page(&snapshot, progress).await;
                if snapshot.state.is_terminal() {
                    let end = match snapshot.state {
                        RemoteAgentJobState::Succeeded => RemoteJobEnd::Succeeded {
                            final_text: text.finish(),
                        },
                        RemoteAgentJobState::Cancelled => RemoteJobEnd::Cancelled {
                            partial_text: text.finish(),
                        },
                        _ => RemoteJobEnd::Failed {
                            error: snapshot
                                .last_error
                                .clone()
                                .unwrap_or_else(|| "the job failed on its device".to_string()),
                        },
                    };
                    return (end, cursor);
                }
                if snapshot.events.is_empty() {
                    OBSERVE_INTERVAL
                } else {
                    Duration::ZERO
                }
            }
            Err(error) => {
                debug!(
                    "Remote agent job observation failed; retrying: job_id={}, error={}",
                    link.job.job_id, error.message
                );
                let wait = retry;
                retry = (retry * 2).min(MAX_RETRY_INTERVAL);
                wait
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = cancel.cancelled() => {}
        }
    }
}

fn spawn_cancel(port: Arc<dyn RemoteAgentDelegationPort>, job: RemoteAgentJob) {
    tokio::spawn(async move {
        if let Err(error) = port.cancel(&job).await {
            warn!(
                "Failed to cancel remote agent job: job_id={}, error={}",
                job.job_id, error.message
            );
        }
    });
}

/// Projects progress onto the child session and forwards prompts.
struct LiveSink {
    holds: HoldTracker,
    stream: ChildSessionStream,
    event_queue: Arc<EventQueue>,
    prompts: PromptForwarder,
}

impl LiveSink {
    async fn enqueue(&self, events: impl IntoIterator<Item = AgenticEvent>) {
        for event in events {
            let _ = self
                .event_queue
                .enqueue(event, Some(EventPriority::Normal))
                .await;
        }
    }
}

#[async_trait::async_trait]
impl JobPageSink for LiveSink {
    async fn on_page(
        &mut self,
        snapshot: &RemoteAgentJobSnapshot,
        progress: Vec<ExternalAgentTurnEvent>,
    ) {
        let projected: Vec<AgenticEvent> = progress
            .into_iter()
            .flat_map(|event| self.stream.project(event))
            .collect();
        self.enqueue(projected).await;
        if let Some(note) = self.holds.observe(snapshot) {
            let projected = self.stream.project(note);
            self.enqueue(projected).await;
        }
        let round_id = if snapshot.pending_permissions.is_empty() {
            String::new()
        } else {
            let (opened, round_id) = self.stream.ensure_round();
            self.enqueue(opened).await;
            round_id
        };
        if let Some(note) = self.prompts.owner_wait_note(&snapshot.pending_permissions) {
            let projected = self.stream.project(note);
            self.enqueue(projected).await;
        }
        self.prompts
            .sync(&snapshot.pending_permissions, round_id)
            .await;
    }
}

/// The job session's final answer: the text of its last round that has any,
/// within its latest turn.
#[derive(Debug, Default)]
pub(crate) struct FinalText {
    current: String,
    last: String,
}

impl FinalText {
    /// Reads one page of raw dispatch events, keeps the answer text, and
    /// returns the job session's progress for projection. Nested sessions on
    /// the target and events this build does not know are skipped.
    pub(crate) fn absorb(
        &mut self,
        raw_events: &[serde_json::Value],
        job_session_id: &str,
    ) -> Vec<ExternalAgentTurnEvent> {
        let mut projected = Vec::new();
        for raw in raw_events {
            if raw.get("type").and_then(serde_json::Value::as_str) != Some("agentEvent") {
                continue;
            }
            let Some(event) = raw
                .get("event")
                .and_then(|envelope| envelope.get("event"))
                .and_then(|event| serde_json::from_value::<AgenticEvent>(event.clone()).ok())
            else {
                continue;
            };
            if event.session_id() != Some(job_session_id) {
                continue;
            }
            match event {
                AgenticEvent::DialogTurnStarted { .. } => {
                    self.current.clear();
                    self.last.clear();
                }
                AgenticEvent::ModelRoundStarted {
                    round_id,
                    round_index,
                    ..
                } => {
                    if !self.current.trim().is_empty() {
                        self.last = std::mem::take(&mut self.current);
                    }
                    self.current.clear();
                    projected.push(ExternalAgentTurnEvent::RoundStarted {
                        round_id,
                        round_index,
                        disable_explore_grouping: false,
                    });
                }
                AgenticEvent::TextChunk { text, .. } => {
                    self.current.push_str(&text);
                    projected.push(ExternalAgentTurnEvent::Text { text });
                }
                AgenticEvent::ThinkingChunk { content, .. } => {
                    projected.push(ExternalAgentTurnEvent::Thought { text: content });
                }
                AgenticEvent::ToolEvent { tool_event, .. } => {
                    projected.push(ExternalAgentTurnEvent::Tool { event: tool_event });
                }
                _ => {}
            }
        }
        projected
    }

    pub(crate) fn finish(&self) -> String {
        if self.current.trim().is_empty() {
            self.last.clone()
        } else {
            self.current.clone()
        }
    }
}

/// Files the job's pending prompts in this host's permission mailbox and
/// sends each answer to the device. A job of another account is answered
/// only by that account's owner on their device, so its prompts are never
/// filed here; the child session says it is waiting instead.
struct PromptForwarder {
    route: Option<ForwardRoute>,
    /// Target request id to local request id.
    forwarded: HashMap<String, String>,
    /// Names the target owner while a job of another account waits.
    owner_wait: Option<OwnerWait>,
}

struct OwnerWait {
    device_name: String,
    /// Target request ids the child session already said it waits for.
    noted: std::collections::HashSet<String>,
}

/// Shown in a cross-account child session while the target owner decides.
pub(crate) fn owner_approval_note(device_name: &str) -> String {
    format!("Waiting for the owner of {device_name} to approve an action on their device.")
}

/// Shown in the child session when the job starts to wait on its device.
pub(crate) fn queued_note(device_name: &str) -> String {
    format!(
        "Queued on {device_name} until a task that its owner allows to run at the same time finishes."
    )
}

/// What the parent hears while a job of another account waits for the
/// owner of its device: the reason and what unblocks it.
fn owner_blocked_reason(device_name: &str) -> String {
    format!(
        "Waiting for the owner of {device_name} to approve an action on their device.\nUnblock: the owner of {device_name} approves or rejects it there."
    )
}

/// What the parent hears while a job waits, queued, to start on its device.
fn queued_reason(device_name: &str, hold: &RemoteAgentJobHold) -> String {
    let allowed = match hold.limit {
        Some(limit) => format!("{limit} task(s)"),
        None => "a limited number of tasks".to_string(),
    };
    match hold.reason {
        openbitfun_runtime_ports::RemoteAgentHoldReason::ConnectionLimit => format!(
            "Queued on {device_name}: its owner allows {allowed} at a time from this connection, and that many are running.\nUnblock: one of them finishes, or the owner of {device_name} raises the limit. The task starts on its own."
        ),
    }
}

/// Keeps the parent informed of why a job makes no progress on its device:
/// queued there, or, for a job of another account, waiting for that
/// account's owner. A job of this account asks this host's own mailbox, so
/// its prompts are not a reason to report.
struct HoldTracker {
    store: Arc<CoordinationStore>,
    session_id: String,
    device_name: String,
    other_account: bool,
    queued: bool,
}

impl HoldTracker {
    fn new(
        store: Arc<CoordinationStore>,
        session_id: &str,
        job: &RemoteAgentJob,
        device_name: String,
    ) -> Self {
        Self {
            store,
            session_id: session_id.to_string(),
            device_name,
            other_account: openbitfun_runtime_ports::remote_target_connection(&job.device_id)
                .is_some(),
            queued: false,
        }
    }

    /// Records the hold this page shows, and returns a note for the child
    /// session when the job starts to queue.
    fn observe(&mut self, snapshot: &RemoteAgentJobSnapshot) -> Option<ExternalAgentTurnEvent> {
        let holds = self.store.remote_holds();
        if let Some(hold) = snapshot
            .hold
            .as_ref()
            .filter(|_| !snapshot.state.is_terminal())
        {
            holds.set(&self.session_id, queued_reason(&self.device_name, hold));
            let started = !self.queued;
            self.queued = true;
            return started.then(|| ExternalAgentTurnEvent::Thought {
                text: queued_note(&self.device_name),
            });
        }
        self.queued = false;
        if self.other_account
            && !snapshot.state.is_terminal()
            && !snapshot.pending_permissions.is_empty()
        {
            holds.set(&self.session_id, owner_blocked_reason(&self.device_name));
        } else {
            holds.clear(&self.session_id);
        }
        None
    }

    fn finish(&self) {
        self.store.remote_holds().clear(&self.session_id);
    }
}

/// The child turn whose prompts a [`PromptForwarder`] files.
struct PromptScope<'a> {
    session_id: &'a str,
    turn_id: &'a str,
    agent_type: &'a str,
    /// Stops waiting for answers when the turn ends.
    cancel: &'a CancellationToken,
}

struct ForwardRoute {
    manager: Arc<openbitfun_agent_runtime::permission::PermissionRequestManager>,
    project_id: String,
    project_path: String,
    delegation: Option<openbitfun_runtime_ports::PermissionDelegationContext>,
    session_id: String,
    turn_id: String,
    agent_type: String,
    device_id: String,
    /// Names the device in the prompt; the device id when it is not listed.
    device_name: String,
    job: RemoteAgentJob,
    port: Arc<dyn RemoteAgentDelegationPort>,
    cancel: CancellationToken,
}

impl PromptForwarder {
    fn new(
        route: Option<&ExternalAgentPermissionRoute>,
        scope: PromptScope<'_>,
        job: &RemoteAgentJob,
        port: &Arc<dyn RemoteAgentDelegationPort>,
        device_name: String,
    ) -> Self {
        if openbitfun_runtime_ports::remote_target_connection(&job.device_id).is_some() {
            return Self {
                route: None,
                forwarded: HashMap::new(),
                owner_wait: Some(OwnerWait {
                    device_name,
                    noted: Default::default(),
                }),
            };
        }
        if route.is_none() {
            warn!(
                "Remote agent prompts cannot be answered from this host: no permission mailbox, agent_type={}",
                scope.agent_type
            );
        }
        Self {
            route: route.map(|route| ForwardRoute {
                manager: route.manager.clone(),
                project_id: route.project_id.clone(),
                project_path: route.project_path.clone(),
                delegation: route.delegation.clone(),
                session_id: scope.session_id.to_string(),
                turn_id: scope.turn_id.to_string(),
                agent_type: scope.agent_type.to_string(),
                device_id: job.device_id.clone(),
                device_name,
                job: job.clone(),
                port: port.clone(),
                cancel: scope.cancel.clone(),
            }),
            forwarded: HashMap::new(),
            owner_wait: None,
        }
    }

    /// For a job of another account: a note when the target asks its owner
    /// something new. Nothing is filed for this host to answer.
    fn owner_wait_note(&mut self, pending: &[PermissionRequest]) -> Option<ExternalAgentTurnEvent> {
        let wait = self.owner_wait.as_mut()?;
        let mut new_request = false;
        for request in pending {
            new_request |= wait.noted.insert(request.request_id.clone());
        }
        new_request.then(|| ExternalAgentTurnEvent::Thought {
            text: owner_approval_note(&wait.device_name),
        })
    }

    async fn sync(&mut self, pending: &[PermissionRequest], round_id: String) {
        {
            let Some(route) = self.route.as_mut() else {
                return;
            };
            // A host that just started may not have listed its devices yet;
            // name the device once it has, before the next prompt is filed.
            if route.device_name == route.device_id && !pending.is_empty() {
                route.device_name = device_name(&route.port, &route.device_id).await;
            }
            let route = &*route;
            let answered: Vec<(String, String)> = self
                .forwarded
                .iter()
                .filter(|(remote_id, _)| {
                    !pending
                        .iter()
                        .any(|request| &request.request_id == *remote_id)
                })
                .map(|(remote_id, local_id)| (remote_id.clone(), local_id.clone()))
                .collect();
            for (remote_id, local_id) in answered {
                self.forwarded.remove(&remote_id);
                let _ = route
                    .manager
                    .cancel_request(&local_id, "Answered on the target device")
                    .await;
            }
            for request in pending {
                if self.forwarded.contains_key(&request.request_id) {
                    continue;
                }
                let local = route.local_request(request, round_id.clone());
                let local_id = local.request_id.clone();
                match route
                    .manager
                    .register_batch_for_turn(vec![local], route.turn_id.clone())
                    .await
                {
                    Ok(mut receivers) => {
                        self.forwarded
                            .insert(request.request_id.clone(), local_id.clone());
                        let Some(receiver) = receivers.pop() else {
                            continue;
                        };
                        let port = route.port.clone();
                        let job = route.job.clone();
                        let remote_id = request.request_id.clone();
                        let cancel = route.cancel.clone();
                        tokio::spawn(async move {
                            let outcome = tokio::select! {
                                outcome = receiver.wait() => outcome,
                                _ = cancel.cancelled() => return,
                            };
                            if let PermissionWaitOutcome::Replied(reply) = outcome {
                                if let Err(error) = port.answer(&job, &remote_id, reply).await {
                                    warn!(
                                        "Failed to send a permission answer to the remote agent job: job_id={}, error={}",
                                        job.job_id, error.message
                                    );
                                }
                            }
                        });
                    }
                    Err(error) => warn!(
                        "Failed to file a remote agent permission prompt: job_id={}, error={}",
                        route.job.job_id, error
                    ),
                }
            }
        }
    }

    async fn cancel_all(&mut self, reason: &str) {
        let Some(route) = self.route.as_ref() else {
            return;
        };
        for (_, local_id) in self.forwarded.drain() {
            let _ = route.manager.cancel_request(&local_id, reason).await;
        }
    }
}

impl ForwardRoute {
    /// The target's prompt, attributed to the child session. Its resources
    /// are the target's, so nothing here can become a local grant.
    fn local_request(&self, request: &PermissionRequest, round_id: String) -> PermissionRequest {
        let mut display_metadata = request.display_metadata.clone();
        display_metadata.insert(
            "remoteDeviceId".into(),
            serde_json::Value::String(self.device_id.clone()),
        );
        display_metadata.insert(
            "remoteDeviceName".into(),
            serde_json::Value::String(self.device_name.clone()),
        );
        display_metadata.insert(
            "remoteAgent".into(),
            serde_json::Value::String(self.agent_type.clone()),
        );
        PermissionRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            round_id,
            order: request.order,
            tool_call_id: request.tool_call_id.clone(),
            project_path: Some(self.project_path.clone()),
            project_id: self.project_id.clone(),
            session_id: self.session_id.clone(),
            agent_id: self.agent_type.clone(),
            action: request.action.clone(),
            resources: request.resources.clone(),
            save_resources: Vec::new(),
            source: PermissionRequestSource {
                kind: PermissionRequestSourceKind::Extension,
                identity: self.agent_type.clone(),
            },
            delegation: self.delegation.clone(),
            display_metadata,
        }
    }
}

async fn device_name(port: &Arc<dyn RemoteAgentDelegationPort>, device_id: &str) -> String {
    port.list_agents(openbitfun_runtime_ports::RemoteAgentListing::Cached)
        .await
        .into_iter()
        .find(|descriptor| descriptor.address.device_id == device_id)
        .map(|descriptor| descriptor.device_name)
        .unwrap_or_else(|| device_id.to_string())
}

fn remote_error(agent_type: &str, error: openbitfun_runtime_ports::PortError) -> OpenBitFunError {
    let message = format!("Remote agent '{agent_type}' failed: {}", error.message);
    match error.kind {
        PortErrorKind::Cancelled => OpenBitFunError::Cancelled(message),
        PortErrorKind::Timeout => OpenBitFunError::Timeout(message),
        PortErrorKind::InvalidRequest => OpenBitFunError::Validation(message),
        _ => OpenBitFunError::service(message),
    }
}

#[cfg(test)]
mod tests;
