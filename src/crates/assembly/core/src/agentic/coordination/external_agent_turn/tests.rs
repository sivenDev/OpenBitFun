use super::*;
use crate::agentic::coordination::coordinator::tests::{
    test_coordinator, test_persistent_coordinator,
};
use crate::agentic::coordination::SubagentExecutionRequest;
use crate::agentic::core::{SessionConfig, SessionContinuationPolicy, SessionModelBindingPolicy};
use crate::agentic::events::EventQueueConfig;
use crate::agentic::tools::pipeline::SubagentParentInfo;
use openbitfun_events::{ToolEventData, ToolEventIdentity};
use openbitfun_runtime_ports::{
    DelegationPolicy, ExternalAgentAvailability, ExternalAgentContext,
    ExternalAgentUnavailableReason, PermissionRuntimeCeiling, SubagentContextMode,
};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::Notify;

/// Scripted executor: emits `events`, then completes, fails, or waits for
/// cancellation.
#[derive(Default)]
struct ScriptedExecutor {
    unavailable: Option<ExternalAgentAvailability>,
    events: Vec<ExternalAgentTurnEvent>,
    error: Option<PortError>,
    block_until_cancel: bool,
    cancel_signal: Notify,
    requests: Mutex<Vec<ExternalAgentTurnRequest>>,
    cancels: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl ExternalAgentExecutor for ScriptedExecutor {
    async fn availability(
        &self,
        _client_id: &str,
        _context: &ExternalAgentContext,
    ) -> ExternalAgentAvailability {
        self.unavailable
            .clone()
            .unwrap_or(ExternalAgentAvailability::Available)
    }

    async fn run_turn(
        &self,
        request: ExternalAgentTurnRequest,
        sink: &dyn ExternalAgentEventSink,
    ) -> PortResult<ExternalAgentTurnOutcome> {
        let prompt = request.prompt.clone();
        self.requests.lock().unwrap().push(request);
        let mut streamed = String::new();
        for event in self.events.clone() {
            if let ExternalAgentTurnEvent::Text { text } = &event {
                streamed.push_str(text);
            }
            sink.emit(event).await?;
        }
        if let Some(error) = self.error.clone() {
            return Err(error);
        }
        if self.block_until_cancel {
            self.cancel_signal.notified().await;
            return Ok(ExternalAgentTurnOutcome::Cancelled {
                partial_text: streamed,
            });
        }
        Ok(ExternalAgentTurnOutcome::Completed {
            final_text: format!("{streamed}done: {prompt}"),
        })
    }

    async fn cancel(&self, client_id: &str, openbitfun_session_id: &str) -> PortResult<bool> {
        self.cancels
            .lock()
            .unwrap()
            .push((client_id.to_string(), openbitfun_session_id.to_string()));
        self.cancel_signal.notify_one();
        Ok(true)
    }

    async fn close(&self, _client_id: &str, _openbitfun_session_id: &str) -> PortResult<()> {
        Ok(())
    }
}

fn turn(
    executor: Option<Arc<ScriptedExecutor>>,
    event_queue: Arc<EventQueue>,
    cancel_token: CancellationToken,
) -> ExternalAgentTurn {
    ExternalAgentTurn {
        executor: executor.map(|executor| executor as Arc<dyn ExternalAgentExecutor>),
        event_queue,
        agent_type: "acp:claude-code".to_string(),
        dialog_turn_id: "child-turn".to_string(),
        request: ExternalAgentTurnRequest {
            client_id: "claude-code".to_string(),
            context: ExternalAgentContext {
                openbitfun_session_id: "child-session".to_string(),
                workspace_path: Some("/work/repo".to_string()),
                remote_connection_id: None,
                session_storage_path: None,
            },
            prompt: "fix the parser".to_string(),
            timeout_seconds: None,
        },
        cancel_token,
    }
}

fn queue() -> Arc<EventQueue> {
    Arc::new(EventQueue::new(EventQueueConfig::default()))
}

async fn drained_events(event_queue: &EventQueue) -> Vec<AgenticEvent> {
    event_queue
        .dequeue_batch(100)
        .await
        .into_iter()
        .map(|envelope| envelope.event)
        .collect()
}

fn final_text(result: &ExecutionResult) -> &str {
    match &result.final_message.content {
        crate::agentic::core::MessageContent::Text(text) => text,
        other => panic!("unexpected final message content: {other:?}"),
    }
}

fn text(text: &str) -> ExternalAgentTurnEvent {
    ExternalAgentTurnEvent::Text {
        text: text.to_string(),
    }
}

fn round(round_id: &str, round_index: usize) -> ExternalAgentTurnEvent {
    ExternalAgentTurnEvent::RoundStarted {
        round_id: round_id.to_string(),
        round_index,
        disable_explore_grouping: false,
    }
}

#[test]
fn client_id_is_parsed_only_from_acp_agent_types() {
    assert_eq!(
        external_agent_client_id("acp:claude-code"),
        Some("claude-code")
    );
    assert_eq!(external_agent_client_id("acp: codex "), Some("codex"));
    assert_eq!(external_agent_client_id("acp:"), None);
    assert_eq!(external_agent_client_id("Explore"), None);
    assert_eq!(external_agent_client_id("ACP:claude-code"), None);
}

#[tokio::test]
async fn completed_turn_streams_child_session_events_and_returns_final_text() {
    let event_queue = queue();
    let executor = Arc::new(ScriptedExecutor {
        events: vec![
            round("r1", 0),
            ExternalAgentTurnEvent::Thought {
                text: "reading".to_string(),
            },
            ExternalAgentTurnEvent::Tool {
                event: ToolEventData::Started {
                    identity: ToolEventIdentity::direct("tool-1".to_string(), "Read"),
                    params: serde_json::json!({}),
                    timeout_seconds: None,
                },
            },
            round("r2", 1),
            text("fixed. "),
            ExternalAgentTurnEvent::ContextUsage { used: 1, size: 2 },
        ],
        ..ScriptedExecutor::default()
    });

    let result = run_external_agent_turn(turn(
        Some(executor.clone()),
        event_queue.clone(),
        CancellationToken::new(),
    ))
    .await
    .expect("turn completes");

    assert!(matches!(result.finish_reason, FinishReason::Complete));
    assert!(result.has_final_response);
    assert!(result.new_messages.is_empty());
    assert_eq!(result.total_rounds, 2);
    assert_eq!(final_text(&result), "fixed. done: fix the parser");
    let request = executor.requests.lock().unwrap()[0].clone();
    assert_eq!(request.context.openbitfun_session_id, "child-session");
    assert_eq!(request.prompt, "fix the parser");

    let events = drained_events(&event_queue).await;
    let shape: Vec<String> = events
        .iter()
        .map(|event| match event {
            AgenticEvent::ModelRoundStarted { round_id, .. } => format!("started:{round_id}"),
            AgenticEvent::ThinkingChunk { round_id, .. } => format!("thinking:{round_id}"),
            AgenticEvent::ToolEvent { round_id, .. } => format!("tool:{round_id}"),
            AgenticEvent::TextChunk { round_id, .. } => format!("text:{round_id}"),
            AgenticEvent::ModelRoundCompleted {
                round_id,
                has_tool_calls,
                ..
            } => format!("completed:{round_id}:{has_tool_calls}"),
            other => format!("unexpected:{other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "started:r1",
            "thinking:r1",
            "tool:r1",
            "completed:r1:true",
            "started:r2",
            "text:r2",
            "completed:r2:false",
        ]
    );
    assert!(events.iter().all(|event| {
        event.session_id() == Some("child-session") && event.turn_id() == Some("child-turn")
    }));
}

#[tokio::test]
async fn content_before_a_round_announcement_opens_a_round() {
    let event_queue = queue();
    let executor = Arc::new(ScriptedExecutor {
        events: vec![text("hello")],
        ..ScriptedExecutor::default()
    });

    let result = run_external_agent_turn(turn(
        Some(executor),
        event_queue.clone(),
        CancellationToken::new(),
    ))
    .await
    .unwrap();

    assert_eq!(result.total_rounds, 1);
    let events = drained_events(&event_queue).await;
    assert!(matches!(
        events.as_slice(),
        [
            AgenticEvent::ModelRoundStarted { round_id: started, .. },
            AgenticEvent::TextChunk { round_id: streamed, .. },
            AgenticEvent::ModelRoundCompleted { round_id: completed, .. },
        ] if started == "child-turn-round-0" && streamed == started && completed == started
    ));
}

#[tokio::test]
async fn cancellation_cancels_the_external_session_and_keeps_partial_text() {
    let executor = Arc::new(ScriptedExecutor {
        events: vec![round("r1", 0), text("half way")],
        block_until_cancel: true,
        ..ScriptedExecutor::default()
    });
    let cancel_token = CancellationToken::new();
    let running = tokio::spawn(run_external_agent_turn(turn(
        Some(executor.clone()),
        queue(),
        cancel_token.clone(),
    )));
    while executor.requests.lock().unwrap().is_empty() {
        tokio::task::yield_now().await;
    }

    cancel_token.cancel();
    let result = running.await.unwrap().expect("partial text is kept");

    assert_eq!(
        executor.cancels.lock().unwrap().as_slice(),
        [("claude-code".to_string(), "child-session".to_string())]
    );
    assert!(matches!(result.finish_reason, FinishReason::Cancelled));
    assert!(!result.has_final_response);
    assert_eq!(final_text(&result), "half way");
}

#[tokio::test]
async fn cancellation_without_output_reports_cancelled() {
    let executor = Arc::new(ScriptedExecutor {
        block_until_cancel: true,
        ..ScriptedExecutor::default()
    });
    let cancel_token = CancellationToken::new();
    cancel_token.cancel();

    let error = run_external_agent_turn(turn(Some(executor.clone()), queue(), cancel_token))
        .await
        .unwrap_err();

    assert!(matches!(error, OpenBitFunError::Cancelled(_)));
    assert_eq!(executor.cancels.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn missing_executor_or_unavailable_client_fails_before_running() {
    let error = run_external_agent_turn(turn(None, queue(), CancellationToken::new()))
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("no external agent executor is registered"));

    let executor = Arc::new(ScriptedExecutor {
        unavailable: Some(ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::DelegationDisabled,
            detail: "delegation is off".to_string(),
        }),
        ..ScriptedExecutor::default()
    });
    let error = run_external_agent_turn(turn(
        Some(executor.clone()),
        queue(),
        CancellationToken::new(),
    ))
    .await
    .unwrap_err();
    assert!(error.to_string().contains("delegation is off"));
    assert!(executor.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn executor_errors_keep_their_kind() {
    let event_queue = queue();
    let executor = Arc::new(ScriptedExecutor {
        events: vec![round("r1", 0)],
        error: Some(PortError::new(PortErrorKind::Timeout, "agent timed out")),
        ..ScriptedExecutor::default()
    });

    let error = run_external_agent_turn(turn(
        Some(executor),
        event_queue.clone(),
        CancellationToken::new(),
    ))
    .await
    .unwrap_err();

    assert!(matches!(error, OpenBitFunError::Timeout(_)));
    // The open round is still closed so the child session does not stay streaming.
    assert!(matches!(
        drained_events(&event_queue).await.last(),
        Some(AgenticEvent::ModelRoundCompleted { .. })
    ));
}

async fn parent_session(
    session_manager: &crate::agentic::session::SessionManager,
) -> (String, tempfile::TempDir) {
    let workspace = tempfile::tempdir().expect("workspace");
    let workspace_path = dunce::canonicalize(workspace.path()).expect("canonical workspace");
    let record =
        crate::service::workspace::legacy_compat::register_local_fixture(&workspace_path, None)
            .await;
    let parent = session_manager
        .create_session(
            "Parent".to_string(),
            "Standard".to_string(),
            SessionConfig {
                model_id: Some("primary".to_string()),
                workspace_path: Some(workspace_path.to_string_lossy().into_owned()),
                workspace_id: Some(record.id),
                ..Default::default()
            },
        )
        .await
        .expect("parent session");
    (parent.session_id, workspace)
}

fn delegation(
    parent_session_id: &str,
    subagent_type: Option<&str>,
    target_session_id: Option<String>,
    prompt: &str,
) -> SubagentExecutionRequest {
    SubagentExecutionRequest {
        task_description: prompt.to_string(),
        requested_agent_id: None,
        context_mode: SubagentContextMode::Fresh,
        target_session_id,
        subagent_type: subagent_type.map(str::to_string),
        logical_subagent_type: subagent_type.map(str::to_string),
        continuation_policy: SessionContinuationPolicy::Reusable,
        model_binding_policy: SessionModelBindingPolicy::Mutable,
        workspace_path: None,
        model_id: None,
        inherit_parent_model: false,
        subagent_parent_info: SubagentParentInfo {
            session_id: parent_session_id.to_string(),
            dialog_turn_id: "parent-turn".to_string(),
            tool_call_id: "task-tool".to_string(),
        },
        context: HashMap::new(),
        permission_runtime_ceiling: PermissionRuntimeCeiling::default(),
        delegation_policy: DelegationPolicy::top_level().spawn_child(),
        external_generation_lease: None,
    }
}

#[tokio::test]
async fn delegated_subagent_turns_run_through_the_executor_and_reuse_the_child_session() {
    // Follow-up input reloads the child from persisted metadata.
    let (coordinator, session_manager) = test_persistent_coordinator();
    let executor = Arc::new(ScriptedExecutor::default());
    coordinator.set_external_agent_executor(executor.clone());
    let (parent_session_id, _workspace) = parent_session(&session_manager).await;

    let first = coordinator
        .execute_subagent(
            delegation(&parent_session_id, Some("acp:claude-code"), None, "fix it"),
            None,
            None,
        )
        .await
        .expect("delegated turn completes");
    let child_session_id = first.session_id().expect("child session").to_string();
    assert_eq!(first.text, "done: fix it");
    let child = session_manager
        .get_session(&child_session_id)
        .expect("child session is kept for follow-up input");
    assert_eq!(child.agent_type, "acp:claude-code");

    let follow_up = coordinator
        .execute_subagent(
            delegation(
                &parent_session_id,
                None,
                Some(child_session_id.clone()),
                "also add a test",
            ),
            None,
            None,
        )
        .await
        .expect("follow-up input completes");

    assert_eq!(follow_up.text, "done: also add a test");
    let requests = executor.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        request.client_id == "claude-code"
            && request.context.openbitfun_session_id == child_session_id
            && request.context.workspace_path.is_some()
    }));
}

#[tokio::test]
async fn delegated_subagent_timeout_cancels_the_external_turn() {
    let (coordinator, session_manager) = test_coordinator();
    let executor = Arc::new(ScriptedExecutor {
        events: vec![round("r1", 0), text("partial findings")],
        block_until_cancel: true,
        ..ScriptedExecutor::default()
    });
    coordinator.set_external_agent_executor(executor.clone());
    let (parent_session_id, _workspace) = parent_session(&session_manager).await;

    let result = coordinator
        .execute_subagent(
            delegation(
                &parent_session_id,
                Some("acp:claude-code"),
                None,
                "slow task",
            ),
            None,
            Some(1),
        )
        .await
        .expect("partial output is returned on timeout");

    assert!(result.is_partial_timeout());
    assert_eq!(result.text, "partial findings");
    assert_eq!(executor.cancels.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn parent_cancellation_cascades_to_the_external_turn() {
    let (coordinator, session_manager) = test_coordinator();
    let executor = Arc::new(ScriptedExecutor {
        block_until_cancel: true,
        ..ScriptedExecutor::default()
    });
    coordinator.set_external_agent_executor(executor.clone());
    let (parent_session_id, _workspace) = parent_session(&session_manager).await;
    let parent_token = CancellationToken::new();

    let cancel_later = {
        let executor = executor.clone();
        let parent_token = parent_token.clone();
        tokio::spawn(async move {
            while executor.requests.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
            parent_token.cancel();
        })
    };
    let error = coordinator
        .execute_subagent(
            delegation(
                &parent_session_id,
                Some("acp:claude-code"),
                None,
                "long task",
            ),
            Some(&parent_token),
            None,
        )
        .await
        .unwrap_err();
    cancel_later.await.unwrap();

    assert!(matches!(error, OpenBitFunError::Cancelled(_)));
    assert_eq!(executor.cancels.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn delegated_subagent_without_a_registered_executor_fails_clearly() {
    let (coordinator, session_manager) = test_coordinator();
    let (parent_session_id, _workspace) = parent_session(&session_manager).await;

    let error = coordinator
        .execute_subagent(
            delegation(&parent_session_id, Some("acp:claude-code"), None, "task"),
            None,
            None,
        )
        .await
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("no external agent executor is registered"));
}

fn remote_binding(connection_id: &str, root: &str) -> WorkspaceBinding {
    WorkspaceBinding::new_remote(
        Some("remote-ws".to_string()),
        std::path::PathBuf::from(root),
        connection_id.to_string(),
        "dev-box".to_string(),
        openbitfun_services_core::workspace_identity::WorkspaceSessionIdentity {
            workspace_kind: openbitfun_core_types::WorkspaceKind::Remote,
            hostname: "dev-box".to_string(),
            logical_workspace_path: root.to_string(),
            remote_connection_id: Some(connection_id.to_string()),
        },
    )
}

#[test]
fn remote_turns_carry_the_ssh_connection_and_posix_root() {
    let binding = remote_binding("ssh-1", "/home/dev/project");
    let context = external_agent_turn_context("acp:claude-code", "child-1", true, Some(&binding))
        .expect("remote context");
    assert_eq!(context.openbitfun_session_id, "child-1");
    assert_eq!(context.remote_connection_id.as_deref(), Some("ssh-1"));
    assert_eq!(context.workspace_path.as_deref(), Some("/home/dev/project"));
}

#[test]
fn local_turns_have_no_connection() {
    let binding = WorkspaceBinding::new(
        Some("local-ws".to_string()),
        std::path::PathBuf::from("/tmp/project"),
    );
    let context = external_agent_turn_context("acp:claude-code", "child-1", false, Some(&binding))
        .expect("local context");
    assert_eq!(context.remote_connection_id, None);
    assert_eq!(context.workspace_path.as_deref(), Some("/tmp/project"));
}

#[test]
fn remote_sessions_without_a_resolved_connection_never_run_locally() {
    let local_binding = WorkspaceBinding::new(None, std::path::PathBuf::from("/home/dev/project"));
    for workspace in [None, Some(&local_binding)] {
        let error = external_agent_turn_context("acp:claude-code", "child-1", true, workspace)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("will not run on this host"),
            "unexpected error: {error}"
        );
    }
}
