//! Provider-neutral boundary for external agents that run turns on behalf of
//! an OpenBitFun session.
//!
//! An external agent (for example an ACP client such as Claude Code) owns its
//! own model loop and tools. OpenBitFun only binds one external session to one
//! OpenBitFun session, sends it a prompt per turn, and projects the streamed
//! progress. Protocol details, process supervision, and client configuration
//! stay in the implementing adapter.

use crate::PortResult;
use async_trait::async_trait;
use openbitfun_events::ToolEventData;
use serde::{Deserialize, Serialize};

/// Why an external agent cannot run in the requested context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentUnavailableReason {
    /// No executor is registered by the current host.
    ExecutorMissing,
    /// The client is not configured or not enabled.
    NotEnabled,
    /// The client is enabled but not allowed to receive delegated work.
    DelegationDisabled,
    /// The client command or its adapter could not be found or probed.
    NotInstalled,
    /// The current execution context (for example a delivery profile or a
    /// remote workspace) cannot host this client.
    UnsupportedContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ExternalAgentAvailability {
    Available,
    Unavailable {
        reason: ExternalAgentUnavailableReason,
        detail: String,
    },
}

impl ExternalAgentAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// How much work `list_agents` may do to report availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentListing {
    /// Configuration facts only; cheap enough to run on every turn.
    Configured,
    /// Also probes client commands so missing installs are reported.
    Probed,
}

/// One external agent the host can offer for delegation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentDescriptor {
    pub client_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_for: Option<String>,
    #[serde(default)]
    pub readonly: bool,
    pub availability: ExternalAgentAvailability,
}

/// Where a turn runs and which OpenBitFun session owns the external session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentContext {
    pub openbitfun_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    /// Saved SSH connection id; absent denotes a local workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_storage_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentTurnRequest {
    pub client_id: String,
    pub context: ExternalAgentContext,
    pub prompt: String,
    /// `None` or zero waits without a fixed timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

/// Progress streamed while a turn runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExternalAgentTurnEvent {
    RoundStarted {
        round_id: String,
        round_index: usize,
        disable_explore_grouping: bool,
    },
    Text {
        text: String,
    },
    Thought {
        text: String,
    },
    Tool {
        event: ToolEventData,
    },
    ContextUsage {
        used: u64,
        size: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ExternalAgentTurnOutcome {
    /// The external agent finished the turn; `final_text` is its answer.
    Completed { final_text: String },
    /// The turn was cancelled; `partial_text` is what arrived before it stopped.
    Cancelled { partial_text: String },
}

/// A permission prompt raised by the external agent's own tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentPermissionRequest {
    /// The external agent's tool call, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// What the agent wants to do, as the agent describes it.
    pub title: String,
    pub kind: ExternalAgentPermissionKind,
    /// Paths or commands the request touches; may be empty.
    #[serde(default)]
    pub resources: Vec<String>,
}

/// Coarse kind of a permission prompt, for presentation and policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentPermissionKind {
    Read,
    Edit,
    Execute,
    Fetch,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentPermissionDecision {
    AllowOnce,
    AllowAlways,
    Reject,
    /// No answer will come, for example because the turn was cancelled.
    Cancelled,
}

#[async_trait]
pub trait ExternalAgentEventSink: Send + Sync {
    async fn emit(&self, event: ExternalAgentTurnEvent) -> PortResult<()>;

    /// Asks whoever drives the OpenBitFun session to answer a permission
    /// prompt. The answer may come from any surface (desktop, mobile, IM bot,
    /// or a peer controller), so it can take arbitrarily long. A sink that
    /// cannot route prompts returns `NotAvailable`.
    async fn request_permission(
        &self,
        _request: ExternalAgentPermissionRequest,
    ) -> PortResult<ExternalAgentPermissionDecision> {
        Err(crate::PortError::new(
            crate::PortErrorKind::NotAvailable,
            "this event sink cannot route permission prompts",
        ))
    }
}

#[async_trait]
pub trait ExternalAgentExecutor: Send + Sync {
    /// Reports whether `client_id` can run in `context` without starting it.
    async fn availability(
        &self,
        client_id: &str,
        context: &ExternalAgentContext,
    ) -> ExternalAgentAvailability;

    /// Lists the external agents this executor can run. With a
    /// `remote_connection_id`, probed availability describes that remote
    /// host instead of this one. Per-session checks still go through
    /// `availability`.
    async fn list_agents(
        &self,
        _listing: ExternalAgentListing,
        _remote_connection_id: Option<&str>,
    ) -> Vec<ExternalAgentDescriptor> {
        Vec::new()
    }

    /// Runs one turn. The first turn for an OpenBitFun session creates the
    /// external session; later turns reuse it, so follow-up prompts keep the
    /// earlier turns as context.
    async fn run_turn(
        &self,
        request: ExternalAgentTurnRequest,
        sink: &dyn ExternalAgentEventSink,
    ) -> PortResult<ExternalAgentTurnOutcome>;

    /// Cancels the running turn of the external session bound to
    /// `openbitfun_session_id`. Returns `false` when no turn was running.
    async fn cancel(&self, client_id: &str, openbitfun_session_id: &str) -> PortResult<bool>;

    /// Releases the external session bound to `openbitfun_session_id`.
    async fn close(&self, client_id: &str, openbitfun_session_id: &str) -> PortResult<()>;
}
