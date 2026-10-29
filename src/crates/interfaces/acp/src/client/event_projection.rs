//! Host-neutral projection of one ACP turn onto `agentic://` frontend events.
//!
//! Hosts deliver the projected `(event name, payload)` pairs; they do not
//! interpret ACP stream events themselves.

use openbitfun_events::EventEmitter;
use serde_json::{json, Value};

use super::stream::AcpClientStreamEvent;

/// One projected frontend event.
#[derive(Debug, Clone, PartialEq)]
pub struct AcpProjectedEvent {
    pub name: &'static str,
    pub payload: Value,
}

impl AcpProjectedEvent {
    fn new(name: &'static str, payload: Value) -> Self {
        Self { name, payload }
    }
}

/// Identity of the turn being projected.
#[derive(Debug, Clone)]
pub struct AcpTurnProjectionTarget {
    pub session_id: String,
    pub turn_id: String,
    pub client_id: String,
    /// Parent linkage for a delegated child session; `Value::Null` for a
    /// standalone ACP session.
    pub subagent_parent_info: Value,
}

/// Stateful projector for one turn: tracks the open model round so text and
/// tool events attach to it, and closes it when the turn ends.
#[derive(Debug)]
pub struct AcpTurnEventProjector {
    target: AcpTurnProjectionTarget,
    current_round_id: Option<String>,
    current_round_has_tool_calls: bool,
}

impl AcpTurnEventProjector {
    pub fn new(target: AcpTurnProjectionTarget) -> Self {
        Self {
            target,
            current_round_id: None,
            current_round_has_tool_calls: false,
        }
    }

    pub fn turn_started(
        &self,
        user_input: &str,
        original_user_input: Option<&str>,
    ) -> AcpProjectedEvent {
        AcpProjectedEvent::new(
            "agentic://dialog-turn-started",
            json!({
                "sessionId": self.target.session_id,
                "turnId": self.target.turn_id,
                "turnIndex": null,
                "userInput": user_input,
                "originalUserInput": original_user_input,
                "userMessageMetadata": null,
                "subagentParentInfo": self.target.subagent_parent_info,
            }),
        )
    }

    pub fn turn_failed(&self, error: &str) -> AcpProjectedEvent {
        AcpProjectedEvent::new(
            "agentic://dialog-turn-failed",
            json!({
                "sessionId": self.target.session_id,
                "turnId": self.target.turn_id,
                "error": error,
                "errorCategory": null,
                "errorDetail": null,
                "subagentParentInfo": self.target.subagent_parent_info,
            }),
        )
    }

    /// Projects one stream event. Text, thought, and tool events require an
    /// open model round; receiving them first is a protocol error.
    pub fn project(
        &mut self,
        event: AcpClientStreamEvent,
    ) -> Result<Vec<AcpProjectedEvent>, String> {
        let target = self.target.clone();
        let events = match event {
            AcpClientStreamEvent::ModelRoundStarted {
                round_id,
                round_index,
                disable_explore_grouping,
            } => {
                let mut events = Vec::with_capacity(2);
                if let Some(previous_round_id) = self.current_round_id.take() {
                    events.push(self.round_completed(previous_round_id));
                }
                self.current_round_id = Some(round_id.clone());
                self.current_round_has_tool_calls = false;
                events.push(AcpProjectedEvent::new(
                    "agentic://model-round-started",
                    json!({
                        "sessionId": target.session_id,
                        "turnId": target.turn_id,
                        "roundId": round_id,
                        "roundIndex": round_index,
                        "renderHints": {
                            "disableExploreGrouping": disable_explore_grouping,
                        },
                        "subagentParentInfo": target.subagent_parent_info,
                    }),
                ));
                events
            }
            AcpClientStreamEvent::AgentText(text) => {
                let round_id = self.open_round("text")?;
                vec![AcpProjectedEvent::new(
                    "agentic://text-chunk",
                    json!({
                        "sessionId": target.session_id,
                        "turnId": target.turn_id,
                        "roundId": round_id,
                        "text": text,
                        "subagentParentInfo": target.subagent_parent_info,
                    }),
                )]
            }
            AcpClientStreamEvent::AgentThought(text) => {
                let round_id = self.open_round("thought")?;
                vec![AcpProjectedEvent::new(
                    "agentic://text-chunk",
                    json!({
                        "sessionId": target.session_id,
                        "turnId": target.turn_id,
                        "roundId": round_id,
                        "text": text,
                        "contentType": "thinking",
                        "isThinkingEnd": false,
                        "subagentParentInfo": target.subagent_parent_info,
                    }),
                )]
            }
            AcpClientStreamEvent::ToolEvent(tool_event) => {
                let round_id = self.open_round("tool event")?;
                self.current_round_has_tool_calls = true;
                vec![AcpProjectedEvent::new(
                    "agentic://tool-event",
                    json!({
                        "sessionId": target.session_id,
                        "turnId": target.turn_id,
                        "roundId": round_id,
                        "toolEvent": tool_event,
                        "subagentParentInfo": target.subagent_parent_info,
                    }),
                )]
            }
            AcpClientStreamEvent::ContextUsageUpdated(usage) => vec![AcpProjectedEvent::new(
                "agentic://acp-context-usage-updated",
                json!({
                    "sessionId": target.session_id,
                    "turnId": target.turn_id,
                    "clientId": target.client_id,
                    "used": usage.used,
                    "size": usage.size,
                    "cost": usage.cost,
                    "subagentParentInfo": target.subagent_parent_info,
                }),
            )],
            AcpClientStreamEvent::AvailableCommandsUpdated(commands) => {
                vec![AcpProjectedEvent::new(
                    "agentic://acp-available-commands-updated",
                    json!({
                        "sessionId": target.session_id,
                        "clientId": target.client_id,
                        "commands": commands,
                    }),
                )]
            }
            AcpClientStreamEvent::PlanUpdated(entries) => vec![AcpProjectedEvent::new(
                "agentic://acp-plan-updated",
                json!({
                    "sessionId": target.session_id,
                    "turnId": target.turn_id,
                    "clientId": target.client_id,
                    "entries": entries,
                }),
            )],
            AcpClientStreamEvent::ConfigOptionsUpdated(_) => vec![AcpProjectedEvent::new(
                "agentic://acp-session-options-changed",
                json!({
                    "sessionId": target.session_id,
                    "clientId": target.client_id,
                }),
            )],
            AcpClientStreamEvent::Completed => {
                let mut events = self.close_round();
                events.push(AcpProjectedEvent::new(
                    "agentic://dialog-turn-completed",
                    json!({
                        "sessionId": self.target.session_id,
                        "turnId": self.target.turn_id,
                        "subagentParentInfo": self.target.subagent_parent_info,
                        "partialRecoveryReason": null,
                    }),
                ));
                events
            }
            AcpClientStreamEvent::Cancelled => {
                let mut events = self.close_round();
                events.push(AcpProjectedEvent::new(
                    "agentic://dialog-turn-cancelled",
                    json!({
                        "sessionId": self.target.session_id,
                        "turnId": self.target.turn_id,
                        "subagentParentInfo": self.target.subagent_parent_info,
                    }),
                ));
                events
            }
        };
        Ok(events)
    }

    fn open_round(&self, what: &str) -> Result<String, String> {
        self.current_round_id
            .clone()
            .ok_or_else(|| format!("ACP {what} arrived before model round start"))
    }

    fn close_round(&mut self) -> Vec<AcpProjectedEvent> {
        self.current_round_id
            .take()
            .map(|round_id| vec![self.round_completed(round_id)])
            .unwrap_or_default()
    }

    fn round_completed(&self, round_id: String) -> AcpProjectedEvent {
        AcpProjectedEvent::new(
            "agentic://model-round-completed",
            json!({
                "sessionId": self.target.session_id,
                "turnId": self.target.turn_id,
                "roundId": round_id,
                "hasToolCalls": self.current_round_has_tool_calls,
            }),
        )
    }
}

/// Delivers projected events through a host-neutral emitter.
pub async fn emit_projected_events(
    emitter: &dyn EventEmitter,
    events: Vec<AcpProjectedEvent>,
) -> Result<(), String> {
    for event in events {
        emitter
            .emit(event.name, event.payload)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::session_options::{
        AcpAvailableCommand, AcpPlanEntry, AcpSessionContextUsage,
    };
    use openbitfun_events::{ToolEventData, ToolEventIdentity};

    fn standalone_projector() -> AcpTurnEventProjector {
        AcpTurnEventProjector::new(AcpTurnProjectionTarget {
            session_id: "acp_claude-code_1".to_string(),
            turn_id: "turn-1".to_string(),
            client_id: "claude-code".to_string(),
            subagent_parent_info: Value::Null,
        })
    }

    fn tool_started() -> ToolEventData {
        ToolEventData::Started {
            identity: ToolEventIdentity::direct("tool-1", "Read"),
            params: json!({ "path": "src/lib.rs" }),
            timeout_seconds: None,
        }
    }

    fn project_all(
        projector: &mut AcpTurnEventProjector,
        events: Vec<AcpClientStreamEvent>,
    ) -> Vec<AcpProjectedEvent> {
        events
            .into_iter()
            .flat_map(|event| projector.project(event).expect("projects"))
            .collect()
    }

    fn round_started(round_id: &str, round_index: usize) -> AcpClientStreamEvent {
        AcpClientStreamEvent::ModelRoundStarted {
            round_id: round_id.to_string(),
            round_index,
            disable_explore_grouping: false,
        }
    }

    /// Payloads mirror the inline projection that `start_acp_dialog_turn`
    /// emitted before the projection moved here.
    #[test]
    fn standalone_turn_matches_the_legacy_desktop_event_sequence() {
        let mut projector = standalone_projector();
        let usage = AcpSessionContextUsage {
            used: 10,
            size: 100,
            cost: None,
        };
        let commands = vec![AcpAvailableCommand {
            name: "review".to_string(),
            description: "Review changes".to_string(),
            input_hint: None,
        }];
        let plan = vec![AcpPlanEntry {
            content: "Read code".to_string(),
            priority: "high".to_string(),
            status: "pending".to_string(),
        }];

        let events = project_all(
            &mut projector,
            vec![
                round_started("r0", 0),
                AcpClientStreamEvent::AgentThought("thinking".to_string()),
                AcpClientStreamEvent::ToolEvent(tool_started()),
                round_started("r1", 1),
                AcpClientStreamEvent::AgentText("done".to_string()),
                AcpClientStreamEvent::ContextUsageUpdated(usage),
                AcpClientStreamEvent::AvailableCommandsUpdated(commands.clone()),
                AcpClientStreamEvent::PlanUpdated(plan.clone()),
                AcpClientStreamEvent::ConfigOptionsUpdated(Vec::new()),
                AcpClientStreamEvent::Completed,
            ],
        );

        let expected = vec![
            AcpProjectedEvent::new(
                "agentic://model-round-started",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "roundId": "r0",
                    "roundIndex": 0,
                    "renderHints": { "disableExploreGrouping": false },
                    "subagentParentInfo": null,
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://text-chunk",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "roundId": "r0",
                    "text": "thinking",
                    "contentType": "thinking",
                    "isThinkingEnd": false,
                    "subagentParentInfo": null,
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://tool-event",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "roundId": "r0",
                    "toolEvent": serde_json::to_value(tool_started()).unwrap(),
                    "subagentParentInfo": null,
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://model-round-completed",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "roundId": "r0",
                    "hasToolCalls": true,
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://model-round-started",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "roundId": "r1",
                    "roundIndex": 1,
                    "renderHints": { "disableExploreGrouping": false },
                    "subagentParentInfo": null,
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://text-chunk",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "roundId": "r1",
                    "text": "done",
                    "subagentParentInfo": null,
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://acp-context-usage-updated",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "clientId": "claude-code",
                    "used": 10,
                    "size": 100,
                    "cost": null,
                    "subagentParentInfo": null,
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://acp-available-commands-updated",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "clientId": "claude-code",
                    "commands": serde_json::to_value(&commands).unwrap(),
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://acp-plan-updated",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "clientId": "claude-code",
                    "entries": serde_json::to_value(&plan).unwrap(),
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://acp-session-options-changed",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "clientId": "claude-code",
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://model-round-completed",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "roundId": "r1",
                    "hasToolCalls": false,
                }),
            ),
            AcpProjectedEvent::new(
                "agentic://dialog-turn-completed",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "subagentParentInfo": null,
                    "partialRecoveryReason": null,
                }),
            ),
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn cancelled_turn_closes_the_open_round_first() {
        let mut projector = standalone_projector();
        let events = project_all(
            &mut projector,
            vec![round_started("r0", 0), AcpClientStreamEvent::Cancelled],
        );
        let names: Vec<_> = events.iter().map(|event| event.name).collect();
        assert_eq!(
            names,
            vec![
                "agentic://model-round-started",
                "agentic://model-round-completed",
                "agentic://dialog-turn-cancelled",
            ]
        );
        assert_eq!(
            events[2].payload,
            json!({
                "sessionId": "acp_claude-code_1",
                "turnId": "turn-1",
                "subagentParentInfo": null,
            })
        );
    }

    #[test]
    fn content_before_a_round_is_a_protocol_error() {
        let mut projector = standalone_projector();
        assert_eq!(
            projector
                .project(AcpClientStreamEvent::AgentText("early".to_string()))
                .unwrap_err(),
            "ACP text arrived before model round start"
        );
        assert_eq!(
            projector
                .project(AcpClientStreamEvent::ToolEvent(tool_started()))
                .unwrap_err(),
            "ACP tool event arrived before model round start"
        );
    }

    #[test]
    fn turn_lifecycle_payloads_match_the_legacy_desktop_shape() {
        let projector = standalone_projector();
        assert_eq!(
            projector.turn_started("hello", Some("hello raw")),
            AcpProjectedEvent::new(
                "agentic://dialog-turn-started",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "turnIndex": null,
                    "userInput": "hello",
                    "originalUserInput": "hello raw",
                    "userMessageMetadata": null,
                    "subagentParentInfo": null,
                }),
            )
        );
        assert_eq!(
            projector.turn_failed("boom"),
            AcpProjectedEvent::new(
                "agentic://dialog-turn-failed",
                json!({
                    "sessionId": "acp_claude-code_1",
                    "turnId": "turn-1",
                    "error": "boom",
                    "errorCategory": null,
                    "errorDetail": null,
                    "subagentParentInfo": null,
                }),
            )
        );
    }

    #[test]
    fn child_session_payloads_carry_the_parent_linkage() {
        let parent = json!({ "toolCallId": "call-1", "sessionId": "parent", "dialogTurnId": "pt" });
        let mut projector = AcpTurnEventProjector::new(AcpTurnProjectionTarget {
            session_id: "child".to_string(),
            turn_id: "turn-1".to_string(),
            client_id: "claude-code".to_string(),
            subagent_parent_info: parent.clone(),
        });
        let events = project_all(
            &mut projector,
            vec![
                round_started("r0", 0),
                AcpClientStreamEvent::AgentText("hi".to_string()),
            ],
        );
        assert!(events
            .iter()
            .all(|event| event.payload["subagentParentInfo"] == parent));
    }
}
