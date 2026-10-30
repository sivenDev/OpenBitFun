use anyhow::{Context, Result};

use openbitfun_core::product_assembly::DeliveryProfile;
use openbitfun_core::product_runtime::CoreRuntimeServicesProvider;
use openbitfun_core::runtime_ownership::CoreRuntimeOwnership;
use openbitfun_core::runtime_ports::{
    ExternalAgentAvailability, ExternalAgentContext, ExternalAgentEventSink, ExternalAgentExecutor,
    ExternalAgentTurnOutcome, ExternalAgentTurnRequest, ExternalAgentUnavailableReason, PortError,
    PortErrorKind, PortResult,
};
use std::sync::{Arc, OnceLock};

pub(crate) use openbitfun_core::agentic::system::AgenticSystem;

/// Set once for processes that must not run external agents, such as a
/// Detached Dispatch worker: the job runs headless with no guaranteed
/// controller, so an external agent's own permission prompts could not be
/// answered.
static EXTERNAL_AGENTS_UNSUPPORTED: OnceLock<&'static str> = OnceLock::new();

pub(crate) const DISPATCH_EXTERNAL_AGENTS_UNSUPPORTED: &str =
    "External agents are not available in Detached Dispatch jobs; delegate to an OpenBitFun agent or run the task from an interactive session";

/// Set once for a Dispatch worker whose job was delegated to an opened
/// `acp:` agent: the job session itself runs that agent, and its prompts go
/// to the job's approval mailbox.
static PRIMARY_EXTERNAL_AGENT_TURNS: OnceLock<()> = OnceLock::new();

/// Makes every external agent in this process report `reason` instead of
/// running. Call before `init_agentic_system`.
pub(crate) fn disable_external_agents_for_process(reason: &'static str) {
    let _ = EXTERNAL_AGENTS_UNSUPPORTED.set(reason);
}

/// Lets top-level sessions of this process run `acp:` agents. Call before
/// `init_agentic_system`, and only for a job delegated to such an agent.
pub(crate) fn allow_primary_external_agent_turns_for_process() {
    let _ = PRIMARY_EXTERNAL_AGENT_TURNS.set(());
}

/// This CLI's executor for external agents, when its ACP client service
/// builds. It answers availability without starting the runtime.
pub(crate) async fn external_agent_executor(
) -> Option<Arc<dyn openbitfun_runtime_ports::ExternalAgentExecutor>> {
    match crate::acp_cli::new_client_service().await {
        Ok(service) => Some(Arc::new(
            openbitfun_acp::client::AcpExternalAgentExecutor::new(service),
        )),
        Err(error) => {
            tracing::warn!("External agent executor unavailable: {}", error);
            None
        }
    }
}

/// Answers every external agent request with an explicit unsupported state.
struct UnsupportedExternalAgentExecutor {
    reason: &'static str,
}

#[async_trait::async_trait]
impl ExternalAgentExecutor for UnsupportedExternalAgentExecutor {
    async fn availability(
        &self,
        _client_id: &str,
        _context: &ExternalAgentContext,
    ) -> ExternalAgentAvailability {
        ExternalAgentAvailability::Unavailable {
            reason: ExternalAgentUnavailableReason::UnsupportedContext,
            detail: self.reason.to_string(),
        }
    }

    async fn run_turn(
        &self,
        _request: ExternalAgentTurnRequest,
        _sink: &dyn ExternalAgentEventSink,
    ) -> PortResult<ExternalAgentTurnOutcome> {
        Err(PortError::new(PortErrorKind::NotAvailable, self.reason))
    }

    async fn cancel(&self, _client_id: &str, _openbitfun_session_id: &str) -> PortResult<bool> {
        Ok(false)
    }

    async fn close(&self, _client_id: &str, _openbitfun_session_id: &str) -> PortResult<()> {
        Ok(())
    }
}

pub(crate) fn select_agentic_system_profile(profile: DeliveryProfile) -> Result<()> {
    openbitfun_core::agentic::system::select_agentic_system_profile(profile)
        .context("Failed to select agentic system delivery profile")
}

pub(crate) async fn init_agentic_system(
    profile: DeliveryProfile,
    runtime_ownership: Arc<CoreRuntimeOwnership>,
) -> Result<AgenticSystem> {
    let system =
        openbitfun_core::agentic::system::init_agentic_system_for_profile_with_runtime_ownership(
            profile,
            runtime_ownership,
        )
        .await
        .context("Failed to initialize agentic system")?;
    system
        .coordinator
        .set_terminal_port(CoreRuntimeServicesProvider::terminal_port());
    system
        .coordinator
        .set_remote_exec_port(CoreRuntimeServicesProvider::remote_exec_port());
    if let Some(reason) = EXTERNAL_AGENTS_UNSUPPORTED.get().copied() {
        system
            .coordinator
            .set_external_agent_executor(Arc::new(UnsupportedExternalAgentExecutor { reason }));
        return Ok(system);
    }
    match crate::acp_cli::new_client_service().await {
        Ok(service) => system.coordinator.set_external_agent_executor(Arc::new(
            openbitfun_acp::client::AcpExternalAgentExecutor::new(service),
        )),
        Err(error) => tracing::warn!(
            "External agent executor unavailable: failed to build ACP client service: {}",
            error
        ),
    }
    if PRIMARY_EXTERNAL_AGENT_TURNS.get().is_some() {
        system.coordinator.allow_primary_external_agent_turns();
    }
    Ok(system)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopSink;

    #[async_trait::async_trait]
    impl ExternalAgentEventSink for NoopSink {
        async fn emit(
            &self,
            _event: openbitfun_core::runtime_ports::ExternalAgentTurnEvent,
        ) -> PortResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn dispatch_processes_refuse_external_agents_with_a_reason() {
        let executor = UnsupportedExternalAgentExecutor {
            reason: DISPATCH_EXTERNAL_AGENTS_UNSUPPORTED,
        };
        let context = ExternalAgentContext {
            openbitfun_session_id: "job-session".to_string(),
            workspace_path: Some("/tmp/job".to_string()),
            remote_connection_id: None,
            session_storage_path: None,
        };
        assert_eq!(
            executor.availability("claude-code", &context).await,
            ExternalAgentAvailability::Unavailable {
                reason: ExternalAgentUnavailableReason::UnsupportedContext,
                detail: DISPATCH_EXTERNAL_AGENTS_UNSUPPORTED.to_string(),
            }
        );
        assert!(executor
            .list_agents(
                openbitfun_core::runtime_ports::ExternalAgentListing::Probed,
                None
            )
            .await
            .is_empty());
        let error = executor
            .run_turn(
                ExternalAgentTurnRequest {
                    client_id: "claude-code".to_string(),
                    context,
                    prompt: "task".to_string(),
                    timeout_seconds: None,
                },
                &NoopSink,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, PortErrorKind::NotAvailable);
        assert!(error.message.contains("Detached Dispatch"));
    }
}
