//! ACP implementation of the provider-neutral external agent executor port.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use openbitfun_core::runtime_ports::{
    ExternalAgentAvailability, ExternalAgentContext, ExternalAgentDescriptor,
    ExternalAgentEventSink, ExternalAgentExecutor, ExternalAgentListing, ExternalAgentTurnEvent,
    ExternalAgentTurnOutcome, ExternalAgentTurnRequest, ExternalAgentUnavailableReason, PortError,
    PortErrorKind, PortResult,
};
use openbitfun_core::util::errors::OpenBitFunError;
use tokio::sync::mpsc;

use super::config::{AcpClientInfo, AcpClientRequirementProbe};
use super::manager::AcpClientService;
use super::stream::AcpClientStreamEvent;

pub struct AcpExternalAgentExecutor {
    service: Arc<AcpClientService>,
}

impl AcpExternalAgentExecutor {
    pub fn new(service: Arc<AcpClientService>) -> Self {
        Self { service }
    }

    /// Requirement probes for this host or for `remote_connection_id`.
    async fn requirement_probes(
        &self,
        remote_connection_id: Option<&str>,
    ) -> Vec<AcpClientRequirementProbe> {
        let probes = self
            .service
            .probe_client_requirements(remote_connection_id, false)
            .await
            .unwrap_or_default();
        if remote_connection_id.is_none() || !probes.is_empty() {
            return probes;
        }
        // A remote host that was never probed has no cached snapshot yet.
        self.service
            .probe_client_requirements(remote_connection_id, true)
            .await
            .unwrap_or_default()
    }
}

fn normalized_connection_id(remote_connection_id: Option<&str>) -> Option<&str> {
    remote_connection_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Decides availability from configuration facts alone.
fn configured_availability(
    client_id: &str,
    client: Option<&AcpClientInfo>,
) -> ExternalAgentAvailability {
    let Some(client) = client.filter(|client| client.enabled) else {
        return ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::NotEnabled,
            detail: format!("ACP client '{client_id}' is not enabled"),
        };
    };
    if !client.subagent.enabled {
        return ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::DelegationDisabled,
            detail: format!("ACP client '{client_id}' does not accept delegated work"),
        };
    }
    ExternalAgentAvailability::Available
}

/// Decides availability from configuration and requirement probe facts.
fn availability_from_facts(
    client_id: &str,
    client: Option<&AcpClientInfo>,
    probe: Option<&AcpClientRequirementProbe>,
    remote: bool,
) -> ExternalAgentAvailability {
    let configured = configured_availability(client_id, client);
    if !configured.is_available() {
        return configured;
    }
    match probe {
        Some(probe) if probe.runnable => ExternalAgentAvailability::Available,
        Some(probe) => ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::NotInstalled,
            detail: if probe.notes.is_empty() {
                format!("ACP client '{client_id}' is not runnable")
            } else {
                probe.notes.join("; ")
            },
        },
        None if remote => ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::NotInstalled,
            detail: format!("ACP client '{client_id}' was not found on the remote host"),
        },
        None => ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::NotInstalled,
            detail: format!("ACP client '{client_id}' has no requirement probe"),
        },
    }
}

/// Describes every configured client. Without probes, availability reflects
/// configuration only; with probes, missing local installs are reported too.
fn descriptors_from_facts(
    clients: &[AcpClientInfo],
    probes: Option<&[AcpClientRequirementProbe]>,
    remote: bool,
) -> Vec<ExternalAgentDescriptor> {
    clients
        .iter()
        .map(|client| {
            let availability = match probes {
                Some(probes) => availability_from_facts(
                    &client.id,
                    Some(client),
                    probes.iter().find(|probe| probe.id == client.id),
                    remote,
                ),
                None => configured_availability(&client.id, Some(client)),
            };
            ExternalAgentDescriptor {
                client_id: client.id.clone(),
                name: client.name.clone(),
                description: client.subagent.description.clone(),
                best_for: client.subagent.best_for.clone(),
                readonly: client.readonly,
                availability,
            }
        })
        .collect()
}

fn port_error(error: OpenBitFunError) -> PortError {
    let kind = match &error {
        OpenBitFunError::Cancelled(_) => PortErrorKind::Cancelled,
        OpenBitFunError::Timeout(_) => PortErrorKind::Timeout,
        OpenBitFunError::Tool(message) if message.contains("timed out after") => {
            PortErrorKind::Timeout
        }
        OpenBitFunError::NotFound(_) => PortErrorKind::NotFound,
        OpenBitFunError::SessionInUse { .. } => PortErrorKind::SessionInUse,
        OpenBitFunError::Validation(_) => PortErrorKind::InvalidRequest,
        _ => PortErrorKind::Backend,
    };
    PortError::new(kind, error.to_string())
}

/// Maps one ACP stream event to port events, accumulating the final answer.
/// Returns the turn outcome when the event ends the turn.
#[derive(Debug, Default)]
struct TurnAccumulator {
    final_text: String,
}

impl TurnAccumulator {
    fn apply(
        &mut self,
        event: AcpClientStreamEvent,
    ) -> (
        Option<ExternalAgentTurnEvent>,
        Option<ExternalAgentTurnOutcome>,
    ) {
        match event {
            AcpClientStreamEvent::ModelRoundStarted {
                round_id,
                round_index,
                disable_explore_grouping,
            } => (
                Some(ExternalAgentTurnEvent::RoundStarted {
                    round_id,
                    round_index,
                    disable_explore_grouping,
                }),
                None,
            ),
            AcpClientStreamEvent::AgentText(text) => {
                self.final_text.push_str(&text);
                (Some(ExternalAgentTurnEvent::Text { text }), None)
            }
            AcpClientStreamEvent::AgentThought(text) => {
                (Some(ExternalAgentTurnEvent::Thought { text }), None)
            }
            AcpClientStreamEvent::ToolEvent(event) => {
                (Some(ExternalAgentTurnEvent::Tool { event }), None)
            }
            AcpClientStreamEvent::ContextUsageUpdated(usage) => (
                Some(ExternalAgentTurnEvent::ContextUsage {
                    used: usage.used,
                    size: usage.size,
                }),
                None,
            ),
            // Client-declared session metadata only drives the standalone ACP
            // session UI; delegated turns do not surface it.
            AcpClientStreamEvent::AvailableCommandsUpdated(_)
            | AcpClientStreamEvent::PlanUpdated(_)
            | AcpClientStreamEvent::ConfigOptionsUpdated(_) => (None, None),
            AcpClientStreamEvent::Completed => (
                None,
                Some(ExternalAgentTurnOutcome::Completed {
                    final_text: self.final_text.clone(),
                }),
            ),
            AcpClientStreamEvent::Cancelled => (
                None,
                Some(ExternalAgentTurnOutcome::Cancelled {
                    partial_text: self.final_text.clone(),
                }),
            ),
        }
    }
}

#[async_trait]
impl ExternalAgentExecutor for AcpExternalAgentExecutor {
    async fn availability(
        &self,
        client_id: &str,
        context: &ExternalAgentContext,
    ) -> ExternalAgentAvailability {
        let remote_connection_id =
            normalized_connection_id(context.remote_connection_id.as_deref());
        let clients = match self.service.list_clients().await {
            Ok(clients) => clients,
            Err(error) => {
                return ExternalAgentAvailability::Unavailable {
                    reason: ExternalAgentUnavailableReason::NotEnabled,
                    detail: format!("Failed to read ACP client configuration: {error}"),
                }
            }
        };
        let client = clients.iter().find(|client| client.id == client_id);

        let probes = self.requirement_probes(remote_connection_id).await;
        let probe = probes.iter().find(|probe| probe.id == client_id);

        availability_from_facts(client_id, client, probe, remote_connection_id.is_some())
    }

    async fn list_agents(
        &self,
        listing: ExternalAgentListing,
        remote_connection_id: Option<&str>,
    ) -> Vec<ExternalAgentDescriptor> {
        let remote_connection_id = normalized_connection_id(remote_connection_id);
        let clients = match self.service.list_clients().await {
            Ok(clients) => clients,
            Err(error) => {
                log::warn!("Failed to list ACP clients for external agents: error={error}");
                return Vec::new();
            }
        };
        match listing {
            ExternalAgentListing::Configured => {
                descriptors_from_facts(&clients, None, remote_connection_id.is_some())
            }
            ExternalAgentListing::Probed => {
                let probes = self.requirement_probes(remote_connection_id).await;
                descriptors_from_facts(&clients, Some(&probes), remote_connection_id.is_some())
            }
        }
    }

    async fn run_turn(
        &self,
        request: ExternalAgentTurnRequest,
        sink: &dyn ExternalAgentEventSink,
    ) -> PortResult<ExternalAgentTurnOutcome> {
        let ExternalAgentTurnRequest {
            client_id,
            context,
            prompt,
            timeout_seconds,
        } = request;
        let (sender, mut receiver) = mpsc::unbounded_channel();

        // The ACP stream callback is synchronous; the sink is async. The
        // callback only enqueues, and the drain loop below awaits the sink.
        let run = self.service.prompt_agent_stream(
            &client_id,
            prompt,
            context.workspace_path,
            context.remote_connection_id,
            context.openbitfun_session_id,
            context.session_storage_path.map(PathBuf::from),
            timeout_seconds,
            move |event| {
                // The receiver only closes after the turn future finishes.
                let _ = sender.send(event);
                Ok(())
            },
        );
        let drain = async {
            let mut accumulator = TurnAccumulator::default();
            let mut outcome = None;
            while let Some(event) = receiver.recv().await {
                let (projected, finished) = accumulator.apply(event);
                if let Some(projected) = projected {
                    sink.emit(projected).await?;
                }
                if finished.is_some() {
                    outcome = finished;
                }
            }
            Ok::<_, PortError>((outcome, accumulator.final_text))
        };

        let (run_result, drain_result) = tokio::join!(run, drain);
        run_result.map_err(port_error)?;
        let (outcome, final_text) = drain_result?;
        // A turn that ended without an explicit stop event still completed.
        Ok(outcome.unwrap_or(ExternalAgentTurnOutcome::Completed { final_text }))
    }

    async fn cancel(&self, client_id: &str, openbitfun_session_id: &str) -> PortResult<bool> {
        let _ = client_id;
        self.service
            .cancel_openbitfun_session(openbitfun_session_id)
            .await
            .map_err(port_error)
    }

    async fn close(&self, client_id: &str, openbitfun_session_id: &str) -> PortResult<()> {
        let _ = client_id;
        self.service
            .release_openbitfun_session(openbitfun_session_id)
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::config::{
        AcpClientPermissionMode, AcpClientStatus, AcpClientSubagentConfig, AcpRequirementProbeItem,
    };
    use openbitfun_events::{ToolEventData, ToolEventIdentity};

    fn client(enabled: bool, delegation: bool) -> AcpClientInfo {
        AcpClientInfo {
            id: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            command: "npx".to_string(),
            args: Vec::new(),
            enabled,
            readonly: false,
            subagent: AcpClientSubagentConfig {
                enabled: delegation,
                description: None,
                best_for: None,
            },
            permission_mode: AcpClientPermissionMode::Ask,
            status: AcpClientStatus::Configured,
            tool_name: "acp__claude-code__prompt".to_string(),
            session_count: 0,
        }
    }

    fn probe(runnable: bool, notes: &[&str]) -> AcpClientRequirementProbe {
        AcpClientRequirementProbe {
            id: "claude-code".to_string(),
            tool: AcpRequirementProbeItem {
                name: "claude".to_string(),
                installed: runnable,
                version: None,
                path: None,
                error: None,
            },
            adapter: None,
            runnable,
            notes: notes.iter().map(|note| note.to_string()).collect(),
        }
    }

    fn reason(availability: ExternalAgentAvailability) -> Option<ExternalAgentUnavailableReason> {
        match availability {
            ExternalAgentAvailability::Available => None,
            ExternalAgentAvailability::Unavailable { reason, .. } => Some(reason),
        }
    }

    #[test]
    fn availability_requires_enabled_delegable_and_runnable_client() {
        let runnable = probe(true, &[]);
        assert_eq!(
            reason(availability_from_facts(
                "claude-code",
                None,
                Some(&runnable),
                false
            )),
            Some(ExternalAgentUnavailableReason::NotEnabled)
        );
        assert_eq!(
            reason(availability_from_facts(
                "claude-code",
                Some(&client(false, true)),
                Some(&runnable),
                false
            )),
            Some(ExternalAgentUnavailableReason::NotEnabled)
        );
        assert_eq!(
            reason(availability_from_facts(
                "claude-code",
                Some(&client(true, false)),
                Some(&runnable),
                false
            )),
            Some(ExternalAgentUnavailableReason::DelegationDisabled)
        );
        assert_eq!(
            availability_from_facts(
                "claude-code",
                Some(&client(true, true)),
                Some(&runnable),
                false
            ),
            ExternalAgentAvailability::Available
        );
    }

    #[test]
    fn descriptors_list_every_client_with_configured_or_probed_availability() {
        let mut described = client(true, true);
        described.subagent.description = Some("Coding agent".to_string());
        described.subagent.best_for = Some("Refactors".to_string());
        let mut disabled = client(false, true);
        disabled.id = "codex".to_string();
        disabled.name = "Codex".to_string();
        let clients = vec![described, disabled];

        let configured = descriptors_from_facts(&clients, None, false);
        assert_eq!(configured.len(), 2);
        assert_eq!(configured[0].client_id, "claude-code");
        assert_eq!(configured[0].name, "Claude Code");
        assert_eq!(configured[0].description.as_deref(), Some("Coding agent"));
        assert_eq!(configured[0].best_for.as_deref(), Some("Refactors"));
        assert_eq!(
            configured[0].availability,
            ExternalAgentAvailability::Available
        );
        assert_eq!(
            reason(configured[1].availability.clone()),
            Some(ExternalAgentUnavailableReason::NotEnabled)
        );

        let probes = vec![probe(false, &["claude is not available on PATH"])];
        let probed = descriptors_from_facts(&clients, Some(&probes), false);
        assert_eq!(
            probed[0].availability,
            ExternalAgentAvailability::Unavailable {
                reason: ExternalAgentUnavailableReason::NotInstalled,
                detail: "claude is not available on PATH".to_string(),
            }
        );
        assert_eq!(
            reason(probed[1].availability.clone()),
            Some(ExternalAgentUnavailableReason::NotEnabled)
        );

        // A remote host without a probe for the client reports it missing
        // there instead of borrowing this host's install.
        let remote = descriptors_from_facts(&clients, Some(&[]), true);
        assert_eq!(
            remote[0].availability,
            ExternalAgentAvailability::Unavailable {
                reason: ExternalAgentUnavailableReason::NotInstalled,
                detail: "ACP client 'claude-code' was not found on the remote host".to_string(),
            }
        );
    }

    #[test]
    fn blank_connection_ids_mean_this_host() {
        assert_eq!(normalized_connection_id(None), None);
        assert_eq!(normalized_connection_id(Some("  ")), None);
        assert_eq!(normalized_connection_id(Some(" ssh-1 ")), Some("ssh-1"));
    }

    #[test]
    fn availability_reports_probe_notes_and_missing_remote_probes() {
        let missing = probe(false, &["claude is not available on PATH"]);
        assert_eq!(
            availability_from_facts(
                "claude-code",
                Some(&client(true, true)),
                Some(&missing),
                false
            ),
            ExternalAgentAvailability::Unavailable {
                reason: ExternalAgentUnavailableReason::NotInstalled,
                detail: "claude is not available on PATH".to_string(),
            }
        );
        assert_eq!(
            availability_from_facts("claude-code", Some(&client(true, true)), None, true),
            ExternalAgentAvailability::Unavailable {
                reason: ExternalAgentUnavailableReason::NotInstalled,
                detail: "ACP client 'claude-code' was not found on the remote host".to_string(),
            }
        );
    }

    #[test]
    fn accumulator_streams_progress_and_returns_the_answer() {
        let mut accumulator = TurnAccumulator::default();
        let (event, done) = accumulator.apply(AcpClientStreamEvent::ModelRoundStarted {
            round_id: "r0".to_string(),
            round_index: 0,
            disable_explore_grouping: false,
        });
        assert!(matches!(
            event,
            Some(ExternalAgentTurnEvent::RoundStarted { .. })
        ));
        assert!(done.is_none());

        accumulator.apply(AcpClientStreamEvent::AgentThought("plan".to_string()));
        let (tool, _) = accumulator.apply(AcpClientStreamEvent::ToolEvent(
            ToolEventData::EarlyDetected {
                identity: ToolEventIdentity::direct("t1", "Edit"),
            },
        ));
        assert!(matches!(tool, Some(ExternalAgentTurnEvent::Tool { .. })));
        accumulator.apply(AcpClientStreamEvent::AgentText("Fixed ".to_string()));
        accumulator.apply(AcpClientStreamEvent::AgentText("it.".to_string()));
        let (metadata, _) = accumulator.apply(AcpClientStreamEvent::PlanUpdated(Vec::new()));
        assert!(metadata.is_none());

        let (_, done) = accumulator.apply(AcpClientStreamEvent::Completed);
        assert_eq!(
            done,
            Some(ExternalAgentTurnOutcome::Completed {
                final_text: "Fixed it.".to_string()
            })
        );
    }

    #[test]
    fn accumulator_reports_cancellation_with_partial_text() {
        let mut accumulator = TurnAccumulator::default();
        accumulator.apply(AcpClientStreamEvent::AgentText("Half".to_string()));
        let (_, done) = accumulator.apply(AcpClientStreamEvent::Cancelled);
        assert_eq!(
            done,
            Some(ExternalAgentTurnOutcome::Cancelled {
                partial_text: "Half".to_string()
            })
        );
    }

    #[test]
    fn port_errors_keep_cancellation_and_timeout_kinds() {
        assert_eq!(
            port_error(OpenBitFunError::Cancelled("stop".to_string())).kind,
            PortErrorKind::Cancelled
        );
        assert_eq!(
            port_error(OpenBitFunError::tool(
                "ACP client timed out after 5s".to_string()
            ))
            .kind,
            PortErrorKind::Timeout
        );
        assert_eq!(
            port_error(OpenBitFunError::service("boom".to_string())).kind,
            PortErrorKind::Backend
        );
    }
}
