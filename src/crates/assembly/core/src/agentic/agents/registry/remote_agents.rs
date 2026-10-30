//! Registry projection of agents that other devices of this account, or paired
//! devices of other accounts, opened to this device for delegation.
//!
//! These entries are not registry definitions: the target device owns their
//! agent, model, tools, and permission approval, so they are listed read-only
//! under the `remote:<deviceId>/<openAgentId>` type that AgentSpawn delegates
//! to as a detached dispatch job.

use super::types::{AgentInfo, AgentSource, RemoteAgentInfo, SubAgentSource};
use crate::agentic::agents::SubagentVisibilityPolicy;
use openbitfun_runtime_ports::{RemoteAgentDescriptor, RemoteAgentTaskKind};

pub(crate) const REMOTE_EXECUTION_KIND: &str = "remote";

/// Converts one host descriptor into a read-only subagent entry.
pub(crate) fn remote_agent_info(descriptor: RemoteAgentDescriptor) -> AgentInfo {
    let id = descriptor.address.agent_type();
    let description = remote_agent_description(&descriptor);
    let effective_enabled = descriptor.availability.is_available();
    AgentInfo {
        key: id.clone(),
        prompt_cache_scope_key: id.clone(),
        id,
        name: descriptor.display_name,
        description,
        is_readonly: false,
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
        external_provider_label: Some(match &descriptor.account {
            Some(account) => format!("{} · {}", account.display_name, descriptor.device_name),
            None => descriptor.device_name.clone(),
        }),
        supports_follow_up: true,
        execution_kind: Some(REMOTE_EXECUTION_KIND.to_string()),
        external_availability: None,
        remote_agent: Some(RemoteAgentInfo {
            device_id: descriptor.address.device_id,
            device_name: descriptor.device_name,
            open_agent_id: descriptor.address.open_agent_id,
            agent_type: descriptor.agent_type,
            task_kinds: descriptor.task_kinds,
            availability: descriptor.availability,
            account: descriptor.account,
        }),
    }
}

/// The model reads this description to pick an agent and task kind, so it
/// names the device and what the agent accepts.
fn remote_agent_description(descriptor: &RemoteAgentDescriptor) -> String {
    let mut description = descriptor
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match &descriptor.account {
            Some(account) => format!(
                "Agent {} that {} opened to this device from another account.",
                descriptor.display_name, account.display_name
            ),
            None => format!(
                "Agent {} that another device of this account opened for delegation.",
                descriptor.display_name
            ),
        });
    match &descriptor.account {
        Some(account) => description.push_str(&format!(
            "\nRuns on device {} of another account ({}, {}) with its own tools, model, and files; it cannot see this workspace, and its owner approves its actions.",
            descriptor.device_name, account.display_name, account.user_id
        )),
        None => description.push_str(&format!(
            "\nRuns on device {} with its own tools, model, and files; it cannot see this workspace.",
            descriptor.device_name
        )),
    }
    let kinds: Vec<&str> = descriptor
        .task_kinds
        .iter()
        .map(|kind| match kind {
            RemoteAgentTaskKind::Message => "message",
            RemoteAgentTaskKind::Coding => "coding",
        })
        .collect();
    if !kinds.is_empty() {
        description.push_str("\nTask kinds: ");
        description.push_str(&kinds.join(", "));
    }
    description
}

/// Appends remote agents, skipping unavailable ones unless disabled entries
/// are requested and never shadowing an existing agent id.
pub(crate) fn merge_remote_agents(
    mut result: Vec<AgentInfo>,
    descriptors: Vec<RemoteAgentDescriptor>,
    include_disabled: bool,
) -> Vec<AgentInfo> {
    for descriptor in descriptors {
        if !include_disabled && !descriptor.availability.is_available() {
            continue;
        }
        let info = remote_agent_info(descriptor);
        if result.iter().any(|existing| existing.id == info.id) {
            continue;
        }
        result.push(info);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use openbitfun_runtime_ports::{
        RemoteAgentAddress, RemoteAgentAvailability, RemoteAgentUnavailableReason,
    };

    fn descriptor(open_agent_id: &str, online: bool) -> RemoteAgentDescriptor {
        RemoteAgentDescriptor {
            address: RemoteAgentAddress {
                device_id: "laptop".to_string(),
                open_agent_id: open_agent_id.to_string(),
            },
            device_name: "Laptop".to_string(),
            agent_type: "Standard".to_string(),
            display_name: "Helper".to_string(),
            description: None,
            task_kinds: vec![RemoteAgentTaskKind::Message, RemoteAgentTaskKind::Coding],
            availability: if online {
                RemoteAgentAvailability::Available
            } else {
                RemoteAgentAvailability::Unavailable {
                    reason: RemoteAgentUnavailableReason::Offline,
                    detail: "Laptop is offline".to_string(),
                }
            },
            account: None,
        }
    }

    #[test]
    fn remote_entries_name_the_device_and_task_kinds() {
        let info = remote_agent_info(descriptor("helper", true));
        assert_eq!(info.id, "remote:laptop/helper");
        assert_eq!(info.execution_kind.as_deref(), Some("remote"));
        assert_eq!(info.external_provider_label.as_deref(), Some("Laptop"));
        assert!(info.effective_enabled);
        assert!(info.description.contains("device Laptop"));
        assert!(info.description.contains("Task kinds: message, coding"));
        let remote = info.remote_agent.expect("remote facts");
        assert_eq!(remote.open_agent_id, "helper");
        assert_eq!(remote.agent_type, "Standard");
    }

    #[test]
    fn connection_entries_name_the_other_account() {
        let mut other = descriptor("helper", true);
        other.address.device_id = openbitfun_runtime_ports::remote_connection_target("c1");
        other.account = Some(openbitfun_runtime_ports::RemoteAgentAccount {
            user_id: "2002".to_string(),
            display_name: "Bea".to_string(),
        });
        let info = remote_agent_info(other);
        assert_eq!(info.id, "remote:conn:c1/helper");
        assert_eq!(
            info.external_provider_label.as_deref(),
            Some("Bea · Laptop")
        );
        assert!(info.description.contains("another account (Bea, 2002)"));
        assert!(info.description.contains("its owner approves"));
        let remote = info.remote_agent.expect("remote facts");
        assert_eq!(remote.account.as_ref().unwrap().display_name, "Bea");
        // The same agent of a same-account device is a different entry.
        let own = remote_agent_info(descriptor("helper", true));
        assert_ne!(own.id, remote.device_id);
        assert!(own.remote_agent.unwrap().account.is_none());
    }

    #[test]
    fn unavailable_entries_are_listed_only_for_management() {
        let descriptors = vec![descriptor("helper", true), descriptor("writer", false)];
        let catalog = merge_remote_agents(Vec::new(), descriptors.clone(), false);
        assert_eq!(
            catalog
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>(),
            ["remote:laptop/helper"]
        );

        let management = merge_remote_agents(Vec::new(), descriptors, true);
        assert_eq!(management.len(), 2);
        let offline = management
            .iter()
            .find(|agent| agent.id == "remote:laptop/writer")
            .unwrap();
        assert!(!offline.effective_enabled);
        assert!(!offline
            .remote_agent
            .as_ref()
            .unwrap()
            .availability
            .is_available());
    }

    #[test]
    fn remote_entries_never_shadow_existing_agents() {
        let mut existing = remote_agent_info(descriptor("helper", true));
        existing.name = "Existing".to_string();
        let merged = merge_remote_agents(vec![existing], vec![descriptor("helper", true)], true);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "Existing");
    }
}
