//! Registry projection of external agents offered by the host's external agent
//! executor (for example ACP clients such as Claude Code).
//!
//! These entries are not registry definitions: the executor owns their
//! configuration, model, and tools, so they are listed read-only under the
//! `acp:<client_id>` type that the Task tool delegates to.

use super::types::{AgentInfo, AgentSource, SubAgentSource};
use crate::agentic::agents::SubagentVisibilityPolicy;
use openbitfun_runtime_ports::ExternalAgentDescriptor;

pub(crate) const ACP_EXECUTION_KIND: &str = "acp";
const ACP_PROVIDER_LABEL: &str = "ACP";

/// Converts one executor descriptor into a read-only subagent entry.
pub(crate) fn acp_agent_info(descriptor: ExternalAgentDescriptor) -> AgentInfo {
    let id = format!("acp:{}", descriptor.client_id);
    let description = acp_agent_description(&descriptor);
    let effective_enabled = descriptor.availability.is_available();
    AgentInfo {
        key: id.clone(),
        prompt_cache_scope_key: id.clone(),
        id,
        name: descriptor.name,
        description,
        is_readonly: descriptor.readonly,
        is_review: false,
        tool_count: 0,
        default_tools: Vec::new(),
        config_profile_id: None,
        config_profile_label: None,
        config_profile_member_mode_ids: Vec::new(),
        default_enabled: effective_enabled,
        effective_enabled,
        override_state: None,
        state_reason: None,
        source: AgentSource::External,
        subagent_source: Some(SubAgentSource::External),
        path: None,
        model: None,
        model_is_explicit: Some(false),
        visibility: Some(SubagentVisibilityPolicy::public().summary()),
        external_provider_label: Some(ACP_PROVIDER_LABEL.to_string()),
        supports_follow_up: true,
        execution_kind: Some(ACP_EXECUTION_KIND.to_string()),
        external_availability: Some(descriptor.availability),
        assistant: None,
    }
}

fn acp_agent_description(descriptor: &ExternalAgentDescriptor) -> String {
    let mut description = descriptor
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "External agent {} that works with its own tools, model, and context.",
                descriptor.name
            )
        });
    if let Some(best_for) = descriptor
        .best_for
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        description.push_str("\nBest for: ");
        description.push_str(best_for);
    }
    description
}

/// Which host external agents are probed on for a management listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAgentHost<'a> {
    /// The host running this process.
    Local,
    /// An SSH remote workspace; agents run on that host.
    Remote { connection_id: &'a str },
    /// The workspace cannot run external agents (for example a remote record
    /// without a saved connection). Nothing is listed, so no local agent is
    /// offered in its place.
    Unsupported,
}

impl<'a> ExternalAgentHost<'a> {
    pub fn for_workspace(workspace: Option<&'a crate::service::workspace::WorkspaceInfo>) -> Self {
        match workspace.map(|record| record.filesystem_connection_id()) {
            None | Some(Ok(None)) => Self::Local,
            Some(Ok(Some(connection_id))) => Self::Remote { connection_id },
            Some(Err(_)) => Self::Unsupported,
        }
    }

    pub(crate) fn remote_connection_id(self) -> Option<Option<&'a str>> {
        match self {
            Self::Local => Some(None),
            Self::Remote { connection_id } => Some(Some(connection_id)),
            Self::Unsupported => None,
        }
    }
}
