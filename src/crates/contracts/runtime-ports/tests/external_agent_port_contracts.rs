#![cfg(feature = "external-agent-port")]

use async_trait::async_trait;
use openbitfun_runtime_ports::{
    ExternalAgentAvailability, ExternalAgentContext, ExternalAgentDescriptor,
    ExternalAgentEventSink, ExternalAgentExecutor, ExternalAgentListing, ExternalAgentTurnEvent,
    ExternalAgentTurnOutcome, ExternalAgentTurnRequest, ExternalAgentUnavailableReason, PortError,
    PortErrorKind, PortResult,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::Notify;

/// Fake executor: one external session per OpenBitFun session, created on the
/// first turn and reused afterwards.
#[derive(Default)]
struct FakeExecutor {
    sessions: Mutex<HashMap<String, usize>>,
    created_sessions: Mutex<usize>,
    running: Mutex<Option<String>>,
    block_turn: Option<std::sync::Arc<Notify>>,
    cancelled: Mutex<bool>,
}

#[async_trait]
impl ExternalAgentExecutor for FakeExecutor {
    async fn availability(
        &self,
        client_id: &str,
        context: &ExternalAgentContext,
    ) -> ExternalAgentAvailability {
        if client_id != "claude-code" {
            return ExternalAgentAvailability::Unavailable {
                reason: ExternalAgentUnavailableReason::NotEnabled,
                detail: format!("{client_id} is not enabled"),
            };
        }
        if context.remote_connection_id.is_some() {
            return ExternalAgentAvailability::Unavailable {
                reason: ExternalAgentUnavailableReason::UnsupportedContext,
                detail: "remote host has no claude".to_string(),
            };
        }
        ExternalAgentAvailability::Available
    }

    async fn run_turn(
        &self,
        request: ExternalAgentTurnRequest,
        sink: &dyn ExternalAgentEventSink,
    ) -> PortResult<ExternalAgentTurnOutcome> {
        let availability = self
            .availability(&request.client_id, &request.context)
            .await;
        if !availability.is_available() {
            return Err(PortError::new(
                PortErrorKind::NotAvailable,
                format!("{availability:?}"),
            ));
        }
        let session_id = request.context.openbitfun_session_id.clone();
        let turn_index = {
            let mut sessions = self.sessions.lock().unwrap();
            let turns = sessions.entry(session_id.clone()).or_insert_with(|| {
                *self.created_sessions.lock().unwrap() += 1;
                0
            });
            *turns += 1;
            *turns
        };
        *self.running.lock().unwrap() = Some(session_id);
        sink.emit(ExternalAgentTurnEvent::RoundStarted {
            round_id: format!("round-{turn_index}"),
            round_index: 0,
            disable_explore_grouping: false,
        })
        .await?;
        sink.emit(ExternalAgentTurnEvent::Text {
            text: format!("turn {turn_index}: "),
        })
        .await?;
        if let Some(block) = &self.block_turn {
            block.notified().await;
        }
        *self.running.lock().unwrap() = None;
        if *self.cancelled.lock().unwrap() {
            return Ok(ExternalAgentTurnOutcome::Cancelled {
                partial_text: format!("turn {turn_index}: "),
            });
        }
        sink.emit(ExternalAgentTurnEvent::Text {
            text: request.prompt.clone(),
        })
        .await?;
        Ok(ExternalAgentTurnOutcome::Completed {
            final_text: format!("turn {turn_index}: {}", request.prompt),
        })
    }

    async fn cancel(&self, _client_id: &str, openbitfun_session_id: &str) -> PortResult<bool> {
        let running = self.running.lock().unwrap().as_deref() == Some(openbitfun_session_id);
        if running {
            *self.cancelled.lock().unwrap() = true;
            if let Some(block) = &self.block_turn {
                block.notify_one();
            }
        }
        Ok(running)
    }

    async fn close(&self, _client_id: &str, openbitfun_session_id: &str) -> PortResult<()> {
        self.sessions.lock().unwrap().remove(openbitfun_session_id);
        Ok(())
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<ExternalAgentTurnEvent>>,
}

#[async_trait]
impl ExternalAgentEventSink for RecordingSink {
    async fn emit(&self, event: ExternalAgentTurnEvent) -> PortResult<()> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

fn context(session_id: &str) -> ExternalAgentContext {
    ExternalAgentContext {
        openbitfun_session_id: session_id.to_string(),
        workspace_path: Some("/work/repo".to_string()),
        remote_connection_id: None,
        session_storage_path: None,
    }
}

fn request(session_id: &str, prompt: &str) -> ExternalAgentTurnRequest {
    ExternalAgentTurnRequest {
        client_id: "claude-code".to_string(),
        context: context(session_id),
        prompt: prompt.to_string(),
        timeout_seconds: None,
    }
}

#[tokio::test]
async fn run_turn_streams_progress_and_completes() {
    let executor = FakeExecutor::default();
    let sink = RecordingSink::default();

    let outcome = executor
        .run_turn(request("child-1", "fix the bug"), &sink)
        .await
        .expect("turn runs");

    assert_eq!(
        outcome,
        ExternalAgentTurnOutcome::Completed {
            final_text: "turn 1: fix the bug".to_string()
        }
    );
    let events = sink.events.lock().unwrap();
    assert!(matches!(
        events.first(),
        Some(ExternalAgentTurnEvent::RoundStarted { .. })
    ));
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn follow_up_turn_reuses_the_bound_external_session() {
    let executor = FakeExecutor::default();
    let sink = RecordingSink::default();

    executor
        .run_turn(request("child-1", "first"), &sink)
        .await
        .unwrap();
    let outcome = executor
        .run_turn(request("child-1", "second"), &sink)
        .await
        .unwrap();
    executor
        .run_turn(request("child-2", "other"), &sink)
        .await
        .unwrap();

    assert_eq!(
        outcome,
        ExternalAgentTurnOutcome::Completed {
            final_text: "turn 2: second".to_string()
        }
    );
    assert_eq!(*executor.created_sessions.lock().unwrap(), 2);
}

#[tokio::test]
async fn cancel_stops_a_running_turn_and_reports_idle_sessions() {
    let block = std::sync::Arc::new(Notify::new());
    let executor = std::sync::Arc::new(FakeExecutor {
        block_turn: Some(block.clone()),
        ..FakeExecutor::default()
    });

    assert!(!executor.cancel("claude-code", "child-1").await.unwrap());

    let running = {
        let executor = executor.clone();
        tokio::spawn(async move {
            let sink = RecordingSink::default();
            executor
                .run_turn(request("child-1", "long task"), &sink)
                .await
        })
    };
    while executor.running.lock().unwrap().is_none() {
        tokio::task::yield_now().await;
    }

    assert!(executor.cancel("claude-code", "child-1").await.unwrap());
    let outcome = running.await.unwrap().unwrap();
    assert_eq!(
        outcome,
        ExternalAgentTurnOutcome::Cancelled {
            partial_text: "turn 1: ".to_string()
        }
    );
}

#[tokio::test]
async fn unavailable_clients_fail_before_running() {
    let executor = FakeExecutor::default();
    let sink = RecordingSink::default();

    let mut remote = request("child-1", "task");
    remote.context.remote_connection_id = Some("ssh-1".to_string());
    assert_eq!(
        executor.availability("claude-code", &remote.context).await,
        ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::UnsupportedContext,
            detail: "remote host has no claude".to_string(),
        }
    );
    let error = executor.run_turn(remote, &sink).await.unwrap_err();
    assert_eq!(error.kind, PortErrorKind::NotAvailable);
    assert!(sink.events.lock().unwrap().is_empty());

    let mut disabled = request("child-1", "task");
    disabled.client_id = "codex".to_string();
    assert!(!executor
        .availability("codex", &disabled.context)
        .await
        .is_available());
}

#[test]
fn turn_request_tolerates_unknown_and_missing_optional_fields() {
    // A newer peer may add fields; an older one may omit optional ones.
    let decoded: ExternalAgentTurnRequest = serde_json::from_value(json!({
        "clientId": "claude-code",
        "context": {
            "openbitfunSessionId": "child-1",
            "futureField": true
        },
        "prompt": "task",
        "futureTopLevel": 1
    }))
    .expect("tolerant decode");

    assert_eq!(decoded.context.workspace_path, None);
    assert_eq!(decoded.context.remote_connection_id, None);
    assert_eq!(decoded.timeout_seconds, None);

    let encoded = serde_json::to_value(&decoded).unwrap();
    assert_eq!(
        encoded,
        json!({
            "clientId": "claude-code",
            "context": { "openbitfunSessionId": "child-1" },
            "prompt": "task"
        })
    );
}

#[tokio::test]
async fn executors_without_a_directory_list_no_agents() {
    let executor = FakeExecutor::default();
    assert!(executor
        .list_agents(ExternalAgentListing::Configured)
        .await
        .is_empty());
    assert!(executor
        .list_agents(ExternalAgentListing::Probed)
        .await
        .is_empty());
}

#[test]
fn agent_descriptor_round_trips_and_tolerates_missing_optional_fields() {
    let decoded: ExternalAgentDescriptor = serde_json::from_value(json!({
        "clientId": "claude-code",
        "name": "Claude Code",
        "availability": {
            "state": "unavailable",
            "reason": "not_installed",
            "detail": "claude-code-acp not found"
        },
        "futureField": 1
    }))
    .expect("tolerant decode");

    assert_eq!(decoded.description, None);
    assert_eq!(decoded.best_for, None);
    assert!(!decoded.readonly);
    assert_eq!(
        decoded.availability,
        ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::NotInstalled,
            detail: "claude-code-acp not found".to_string(),
        }
    );

    let encoded = serde_json::to_value(&decoded).unwrap();
    assert_eq!(
        encoded,
        json!({
            "clientId": "claude-code",
            "name": "Claude Code",
            "readonly": false,
            "availability": {
                "state": "unavailable",
                "reason": "not_installed",
                "detail": "claude-code-acp not found"
            }
        })
    );
}
