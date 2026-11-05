//! Delegated subagent turns that run as an assistant another device shares.
//!
//! A subagent whose agent type is `shared:<ownerDeviceId>/<assistantId>` keeps the
//! native background subagent lifecycle (session, cancellation, AgentWait);
//! only the turn body is replaced. The turn submits a detached dispatch job
//! to the device (or continues the session's earlier job), observes its event
//! log by cursor, and projects the job session's progress onto the child
//! session. The job's permission prompts are answered by the device's owner
//! on that device; this session only says it is waiting.

use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{debug, warn};
use openbitfun_events::{RemoteCallHold, RemoteCallHoldReason};
use openbitfun_runtime_ports::{
    ExternalAgentTurnEvent, PortErrorKind, RemoteAgentAddress, RemoteAgentDelegationPort,
    RemoteAgentJob, RemoteAgentJobHold, RemoteAgentJobSnapshot, RemoteAgentJobState,
    RemoteAgentSource, RemoteAgentSubmitRequest,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::coordination_store::CoordinationStore;
use super::external_agent_turn::{execution_result, ChildSessionStream};
use crate::agentic::events::{AgenticEvent, EventPriority, EventQueue};
use crate::agentic::execution::{ExecutionResult, FinishReason};
use crate::util::errors::{OpenBitFunError, OpenBitFunResult};

/// Delay between observations of a running job. A full page is read again
/// at once.
const OBSERVE_INTERVAL: Duration = Duration::from_millis(1500);
/// Upper bound on the delay between retries while the device is unreachable.
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Child context key carrying the call chain of a call to another device's
/// assistant, as a JSON array; set by the Task tool at admission.
pub(crate) const REMOTE_AGENT_CALL_CHAIN_CONTEXT_KEY: &str = "remoteAgentCallChain";

/// The call chain the Task tool admitted the call with.
pub(crate) fn admitted_call_chain(
    context: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    context
        .get(REMOTE_AGENT_CALL_CHAIN_CONTEXT_KEY)
        .and_then(|chain| serde_json::from_str(chain).ok())
        .unwrap_or_default()
}

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
    pub(crate) call_chain: Vec<String>,
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
                    call_chain: turn.call_chain.clone(),
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
            job_device_name,
        ),
        // Rounds name no model until the owner's device reports one; its
        // device id is not a model.
        stream: ChildSessionStream::new(&turn.session_id, &turn.dialog_turn_id, &agent_type, ""),
        event_queue: turn.event_queue.clone(),
    };
    let (end, cursor) = observe_job(&port, &link, &turn.cancel_token, &mut sink).await;
    if let Some(cleared) = sink.holds.finish() {
        sink.enqueue([cleared]).await;
    }
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
    let recorded = match turn
        .coordination_store
        .set_remote_job(&turn.parent_session_id, &turn.dialog_turn_id, link.encode())
        .await
    {
        // A turn the user sent to the child directly has no task of its own;
        // the job's latest record carries its cursor for the next follow-up.
        Ok(false) => {
            turn.coordination_store
                .set_latest_remote_job_for_child(&turn.session_id, link.encode())
                .await
        }
        other => other,
    };
    match recorded {
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

    /// The model the owner's device reports running the job on.
    fn report_model(&mut self, _model: &str) {}
}

/// Observes a task this process adopted after a restart. Its progress was
/// already shown by the process that started it, so only the outcome and
/// what the job waits for are kept. Cancelling the task through
/// AgentControl cancels the job too.
pub(crate) struct ResumedTaskSink {
    holds: HoldTracker,
    store: Arc<CoordinationStore>,
    task_pk: i64,
    cancel: CancellationToken,
    stream: ChildSessionStream,
    event_queue: Arc<EventQueue>,
}

/// The adopted task a [`ResumedTaskSink`] observes.
pub(crate) struct ResumedTask<'a> {
    pub(crate) store: Arc<CoordinationStore>,
    pub(crate) event_queue: Arc<EventQueue>,
    pub(crate) task_pk: i64,
    pub(crate) session_id: &'a str,
    pub(crate) turn_id: &'a str,
    pub(crate) agent_type: &'a str,
}

impl ResumedTaskSink {
    pub(crate) async fn new(
        task: ResumedTask<'_>,
        link: &RemoteJobLink,
        port: &Arc<dyn RemoteAgentDelegationPort>,
        cancel: CancellationToken,
    ) -> Self {
        let job_device_name = device_name(port, &link.job.device_id).await;
        Self {
            holds: HoldTracker::new(task.store.clone(), task.session_id, job_device_name),
            store: task.store,
            task_pk: task.task_pk,
            stream: ChildSessionStream::new(task.session_id, task.turn_id, task.agent_type, ""),
            event_queue: task.event_queue,
            cancel,
        }
    }

    /// Clears what the task waited for once observation ends.
    pub(crate) async fn finish(mut self) {
        if let Some(cleared) = self.holds.finish() {
            self.enqueue(cleared).await;
        }
        if let Some(closed) = self.stream.close_round() {
            let _ = self
                .event_queue
                .enqueue(closed, Some(EventPriority::Normal))
                .await;
        }
    }
}

impl ResumedTaskSink {
    async fn enqueue(&self, event: AgenticEvent) {
        let _ = self
            .event_queue
            .enqueue(event, Some(EventPriority::Normal))
            .await;
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
            if let Some(cleared) = self.holds.finish() {
                self.enqueue(cleared).await;
            }
            self.cancel.cancel();
            return;
        }
        if let Some(changed) = self.holds.observe(snapshot) {
            self.enqueue(changed).await;
        }
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
                if let Some(model) = text.model() {
                    sink.report_model(model);
                }
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

/// Projects progress onto the child session and says what it waits for.
struct LiveSink {
    holds: HoldTracker,
    stream: ChildSessionStream,
    event_queue: Arc<EventQueue>,
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
    fn report_model(&mut self, model: &str) {
        self.stream.report_model(model);
    }

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
        if let Some(changed) = self.holds.observe(snapshot) {
            self.enqueue([changed]).await;
        }
    }
}

/// The job session's final answer: the text of its last round that has any,
/// within its latest turn.
#[derive(Debug, Default)]
pub(crate) struct FinalText {
    current: String,
    last: String,
    /// The model the job session's latest round ran on, as its device names it.
    model: Option<String>,
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
                    effective_model_name,
                    ..
                } => {
                    let model = effective_model_name.trim();
                    if !model.is_empty() {
                        self.model = Some(model.to_string());
                    }
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

    pub(crate) fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub(crate) fn finish(&self) -> String {
        if self.current.trim().is_empty() {
            self.last.clone()
        } else {
            self.current.clone()
        }
    }
}

/// What the parent hears while a job waits for the owner of its device: the
/// reason and what unblocks it.
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
        openbitfun_runtime_ports::RemoteAgentHoldReason::ShareLimit => format!(
            "Queued on {device_name}: its owner allows this assistant {allowed} at a time for other devices, and that many are running.\nUnblock: one of them finishes, or the owner of {device_name} raises the limit. The task starts on its own."
        ),
    }
}

/// Keeps the parent and the caller's view informed of why a job makes no
/// progress on its device: queued there, or waiting for the device's owner.
struct HoldTracker {
    store: Arc<CoordinationStore>,
    session_id: String,
    device_name: String,
    current: Option<RemoteCallHold>,
}

impl HoldTracker {
    fn new(store: Arc<CoordinationStore>, session_id: &str, device_name: String) -> Self {
        Self {
            store,
            session_id: session_id.to_string(),
            device_name,
            current: None,
        }
    }

    /// Records the hold this page shows for the parent, and returns the
    /// event that tells the caller's view when it changes.
    fn observe(&mut self, snapshot: &RemoteAgentJobSnapshot) -> Option<AgenticEvent> {
        let holds = self.store.remote_holds();
        let hold = if snapshot.state.is_terminal() {
            None
        } else if let Some(hold) = snapshot.hold.as_ref() {
            holds.set(&self.session_id, queued_reason(&self.device_name, hold));
            Some(RemoteCallHold {
                reason: match hold.reason {
                    openbitfun_runtime_ports::RemoteAgentHoldReason::ShareLimit => {
                        RemoteCallHoldReason::ShareLimit
                    }
                },
                device_name: self.device_name.clone(),
                limit: hold.limit,
            })
        } else if !snapshot.pending_permissions.is_empty() {
            holds.set(&self.session_id, owner_blocked_reason(&self.device_name));
            Some(RemoteCallHold {
                reason: RemoteCallHoldReason::OwnerApproval,
                device_name: self.device_name.clone(),
                limit: None,
            })
        } else {
            None
        };
        if hold.is_none() {
            holds.clear(&self.session_id);
        }
        self.change_to(hold)
    }

    /// Clears the hold once observation ends, returning the event that says
    /// so when one was shown.
    fn finish(&mut self) -> Option<AgenticEvent> {
        self.store.remote_holds().clear(&self.session_id);
        self.change_to(None)
    }

    fn change_to(&mut self, hold: Option<RemoteCallHold>) -> Option<AgenticEvent> {
        if self.current == hold {
            return None;
        }
        self.current = hold.clone();
        Some(AgenticEvent::RemoteCallHoldChanged {
            session_id: self.session_id.clone(),
            hold,
        })
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
