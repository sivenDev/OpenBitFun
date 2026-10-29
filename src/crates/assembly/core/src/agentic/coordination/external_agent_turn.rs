//! Delegated subagent turns executed by a registered external agent executor.
//!
//! A subagent whose agent type is `acp:<client_id>` keeps the native subagent
//! lifecycle (session, timeout, cancellation, background delivery, AgentWait);
//! only the turn body is replaced. The executor owns the external agent's tool
//! policy, so no OpenBitFun tools or prompts are injected here.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use log::warn;
use openbitfun_runtime_ports::{
    ExternalAgentEventSink, ExternalAgentExecutor, ExternalAgentTurnEvent,
    ExternalAgentTurnOutcome, ExternalAgentTurnRequest, PortError, PortErrorKind, PortResult,
};
use tokio_util::sync::CancellationToken;

use crate::agentic::core::Message;
use crate::agentic::events::{AgenticEvent, EventPriority, EventQueue};
use crate::agentic::execution::{ExecutionResult, FinishReason};
use crate::util::errors::{OpenBitFunError, OpenBitFunResult};

pub(crate) const EXTERNAL_AGENT_TYPE_PREFIX: &str = "acp:";

/// Returns the ACP client id when `agent_type` names an external agent.
pub(crate) fn external_agent_client_id(agent_type: &str) -> Option<&str> {
    agent_type
        .strip_prefix(EXTERNAL_AGENT_TYPE_PREFIX)
        .map(str::trim)
        .filter(|client_id| !client_id.is_empty())
}

/// Projects executor events onto the child session's stream events.
struct ChildSessionStream {
    session_id: String,
    turn_id: String,
    agent_type: String,
    client_id: String,
    current_round: Option<OpenRound>,
    rounds: usize,
}

struct OpenRound {
    round_id: String,
    has_tool_calls: bool,
    started_at: Instant,
}

impl ChildSessionStream {
    fn new(session_id: &str, turn_id: &str, agent_type: &str, client_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            agent_type: agent_type.to_string(),
            client_id: client_id.to_string(),
            current_round: None,
            rounds: 0,
        }
    }

    fn project(&mut self, event: ExternalAgentTurnEvent) -> Vec<AgenticEvent> {
        match event {
            ExternalAgentTurnEvent::RoundStarted {
                round_id,
                round_index,
                ..
            } => {
                let mut events: Vec<AgenticEvent> = self.close_round().into_iter().collect();
                events.push(self.start_round(round_id, round_index));
                events
            }
            ExternalAgentTurnEvent::Text { text } => {
                let (opened, round_id) = self.ensure_round();
                opened
                    .into_iter()
                    .chain(std::iter::once(AgenticEvent::TextChunk {
                        session_id: self.session_id.clone(),
                        turn_id: self.turn_id.clone(),
                        round_id,
                        attempt_id: None,
                        attempt_index: None,
                        text,
                    }))
                    .collect()
            }
            ExternalAgentTurnEvent::Thought { text } => {
                let (opened, round_id) = self.ensure_round();
                opened
                    .into_iter()
                    .chain(std::iter::once(AgenticEvent::ThinkingChunk {
                        session_id: self.session_id.clone(),
                        turn_id: self.turn_id.clone(),
                        round_id,
                        attempt_id: None,
                        attempt_index: None,
                        content: text,
                        reasoning_kind: None,
                        is_end: false,
                    }))
                    .collect()
            }
            ExternalAgentTurnEvent::Tool { event } => {
                let (opened, round_id) = self.ensure_round();
                if let Some(round) = self.current_round.as_mut() {
                    round.has_tool_calls = true;
                }
                opened
                    .into_iter()
                    .chain(std::iter::once(AgenticEvent::ToolEvent {
                        session_id: self.session_id.clone(),
                        turn_id: self.turn_id.clone(),
                        round_id,
                        attempt_id: None,
                        attempt_index: None,
                        tool_event: event,
                    }))
                    .collect()
            }
            // Context usage describes the external agent's own window, not an
            // OpenBitFun model budget.
            ExternalAgentTurnEvent::ContextUsage { .. } => Vec::new(),
        }
    }

    fn start_round(&mut self, round_id: String, round_index: usize) -> AgenticEvent {
        self.current_round = Some(OpenRound {
            round_id: round_id.clone(),
            has_tool_calls: false,
            started_at: Instant::now(),
        });
        self.rounds += 1;
        AgenticEvent::ModelRoundStarted {
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            round_id,
            round_group_id: None,
            round_index,
            model_config_id: self.agent_type.clone(),
            effective_model_name: self.client_id.clone(),
            routing: None,
        }
    }

    /// Opens a round when the executor streams content before announcing one.
    fn ensure_round(&mut self) -> (Option<AgenticEvent>, String) {
        if let Some(round) = self.current_round.as_ref() {
            return (None, round.round_id.clone());
        }
        let round_id = format!("{}-round-{}", self.turn_id, self.rounds);
        let started = self.start_round(round_id.clone(), self.rounds);
        (Some(started), round_id)
    }

    fn close_round(&mut self) -> Option<AgenticEvent> {
        let round = self.current_round.take()?;
        Some(AgenticEvent::ModelRoundCompleted {
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            round_id: round.round_id,
            has_tool_calls: round.has_tool_calls,
            duration_ms: Some(round.started_at.elapsed().as_millis() as u64),
            provider_id: None,
            model_config_id: self.agent_type.clone(),
            effective_model_name: self.client_id.clone(),
            first_chunk_ms: None,
            first_visible_output_ms: None,
            stream_duration_ms: None,
            ttft_ms: None,
            attempt_count: None,
            failure_category: None,
            token_details: None,
        })
    }
}

struct EventQueueSink {
    event_queue: Arc<EventQueue>,
    stream: tokio::sync::Mutex<ChildSessionStream>,
}

impl EventQueueSink {
    async fn enqueue_all(&self, events: Vec<AgenticEvent>) {
        for event in events {
            let _ = self
                .event_queue
                .enqueue(event, Some(EventPriority::Normal))
                .await;
        }
    }

    async fn finish(&self) -> usize {
        let (closed, rounds) = {
            let mut stream = self.stream.lock().await;
            (stream.close_round(), stream.rounds)
        };
        self.enqueue_all(closed.into_iter().collect()).await;
        rounds
    }
}

#[async_trait]
impl ExternalAgentEventSink for EventQueueSink {
    async fn emit(&self, event: ExternalAgentTurnEvent) -> PortResult<()> {
        let events = self.stream.lock().await.project(event);
        self.enqueue_all(events).await;
        Ok(())
    }
}

pub(crate) struct ExternalAgentTurn {
    pub(crate) executor: Option<Arc<dyn ExternalAgentExecutor>>,
    pub(crate) event_queue: Arc<EventQueue>,
    pub(crate) agent_type: String,
    pub(crate) dialog_turn_id: String,
    pub(crate) request: ExternalAgentTurnRequest,
    pub(crate) cancel_token: CancellationToken,
}

/// Runs one delegated turn and shapes its result like a native dialog turn so
/// the coordinator's completion, timeout, and cancellation paths apply as-is.
///
/// The turn reacts to `cancel_token` by asking the executor to cancel and then
/// waits for the executor to settle; partial text is returned so a timeout can
/// still deliver it.
pub(crate) async fn run_external_agent_turn(
    turn: ExternalAgentTurn,
) -> OpenBitFunResult<ExecutionResult> {
    let ExternalAgentTurn {
        executor,
        event_queue,
        agent_type,
        dialog_turn_id,
        request,
        cancel_token,
    } = turn;
    let started_at = Instant::now();
    let executor = executor.ok_or_else(|| {
        OpenBitFunError::service(format!(
            "External agent '{agent_type}' is unavailable: no external agent executor is registered in this host"
        ))
    })?;
    let availability = executor
        .availability(&request.client_id, &request.context)
        .await;
    if let openbitfun_runtime_ports::ExternalAgentAvailability::Unavailable { detail, .. } =
        availability
    {
        return Err(OpenBitFunError::service(format!(
            "External agent '{agent_type}' is unavailable: {detail}"
        )));
    }

    let client_id = request.client_id.clone();
    let session_id = request.context.openbitfun_session_id.clone();
    let sink = EventQueueSink {
        event_queue,
        stream: tokio::sync::Mutex::new(ChildSessionStream::new(
            &session_id,
            &dialog_turn_id,
            &agent_type,
            &client_id,
        )),
    };

    let outcome = {
        let run = executor.run_turn(request, &sink);
        tokio::pin!(run);
        tokio::select! {
            outcome = &mut run => outcome,
            _ = cancel_token.cancelled() => {
                if let Err(error) = executor.cancel(&client_id, &session_id).await {
                    warn!(
                        "Failed to cancel external agent turn: agent_type={}, session_id={}, error={}",
                        agent_type, session_id, error
                    );
                }
                run.await
            }
        }
    };
    let total_rounds = sink.finish().await;
    let duration_ms = started_at.elapsed().as_millis() as u64;

    match outcome.map_err(|error| port_error(&agent_type, error))? {
        ExternalAgentTurnOutcome::Completed { final_text } => Ok(execution_result(
            final_text,
            total_rounds,
            duration_ms,
            FinishReason::Complete,
        )),
        ExternalAgentTurnOutcome::Cancelled { partial_text }
            if cancel_token.is_cancelled() && !partial_text.trim().is_empty() =>
        {
            Ok(execution_result(
                partial_text,
                total_rounds,
                duration_ms,
                FinishReason::Cancelled,
            ))
        }
        ExternalAgentTurnOutcome::Cancelled { .. } => Err(OpenBitFunError::Cancelled(format!(
            "External agent '{agent_type}' turn was cancelled"
        ))),
    }
}

fn execution_result(
    text: String,
    total_rounds: usize,
    duration_ms: u64,
    finish_reason: FinishReason,
) -> ExecutionResult {
    let complete = matches!(finish_reason, FinishReason::Complete);
    ExecutionResult {
        final_message: Message::assistant(text),
        total_rounds,
        success: complete,
        // The external agent keeps its own transcript; persistence records
        // the final answer against the streamed rounds.
        new_messages: Vec::new(),
        finish_reason,
        total_tools: 0,
        duration_ms,
        partial_recovery_reason: (!complete).then(|| "external_agent_cancelled".to_string()),
        effective_finish_reason: if complete { "complete" } else { "cancelled" }.to_string(),
        has_final_response: complete,
    }
}

fn port_error(agent_type: &str, error: PortError) -> OpenBitFunError {
    let message = format!("External agent '{agent_type}' failed: {}", error.message);
    match error.kind {
        PortErrorKind::Cancelled => OpenBitFunError::Cancelled(message),
        PortErrorKind::Timeout => OpenBitFunError::Timeout(message),
        PortErrorKind::NotFound => OpenBitFunError::NotFound(message),
        PortErrorKind::InvalidRequest => OpenBitFunError::Validation(message),
        _ => OpenBitFunError::service(message),
    }
}

#[cfg(test)]
mod tests;
