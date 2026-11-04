//! The assistants a person may name with「@」in a conversation: the ones its
//! next turn could call, under the same rules as the Task catalog.

use super::execution::{parent_refusal, ParentFacts, ParentRefusal};
use super::*;
use crate::agentic::agents::{is_swarm_delegate_agent_type, AgentInfo, AssistantCaller};
use openbitfun_runtime_ports::RemoteAgentAvailability;
use serde::{Deserialize, Serialize};

/// Why a conversation can name no assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallableAssistantsRefusal {
    /// Review sessions hand work to review agents only.
    Review,
    /// The conversation's mode cannot call subagents or external agents.
    Mode,
    /// The conversation runs under permission restrictions an assistant's
    /// own tools cannot be held to.
    PermissionRestrictions,
    Swarm,
    /// A task another device asked for, under a share that does not let it
    /// call assistants.
    Trust,
    /// This host runs no conversation coordinator.
    Unavailable,
}

impl From<ParentRefusal> for CallableAssistantsRefusal {
    fn from(refusal: ParentRefusal) -> Self {
        match refusal {
            ParentRefusal::Review => Self::Review,
            ParentRefusal::Mode => Self::Mode,
            ParentRefusal::PermissionRestrictions => Self::PermissionRestrictions,
            ParentRefusal::Swarm => Self::Swarm,
        }
    }
}

/// Whose assistant a callable entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallableAssistantOwnerKind {
    Mine,
    Shared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallableAssistantOwner {
    pub kind: CallableAssistantOwnerKind,
    /// The device that shares it; absent for this device's own assistants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallableAssistant {
    /// The `subagent_type` a call uses: `assistant:<id>` or
    /// `shared:<ownerDeviceId>/<assistantId>`.
    pub agent_type: String,
    pub name: String,
    pub owner: CallableAssistantOwner,
    pub available: bool,
    /// Why a shared assistant cannot take work now, as
    /// `RemoteAgentUnavailableReason` names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<openbitfun_runtime_ports::RemoteAgentUnavailableReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallableAssistants {
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CallableAssistantsRefusal>,
    #[serde(default)]
    pub assistants: Vec<CallableAssistant>,
}

impl CallableAssistants {
    fn refused(reason: CallableAssistantsRefusal) -> Self {
        Self {
            supported: false,
            reason: Some(reason),
            assistants: Vec::new(),
        }
    }

    /// The list for a conversation that may call assistants, taken from
    /// the Assistant category of its catalog.
    fn listed(category: Vec<AgentInfo>) -> Self {
        Self {
            supported: true,
            reason: None,
            assistants: category
                .into_iter()
                .filter_map(callable_assistant)
                .collect(),
        }
    }
}

fn callable_assistant(info: AgentInfo) -> Option<CallableAssistant> {
    let assistant = info.assistant?;
    let (available, unavailable_reason) = match assistant.availability {
        RemoteAgentAvailability::Available => (true, None),
        RemoteAgentAvailability::Unavailable { reason, .. } => (false, Some(reason)),
    };
    let owner = match assistant.owner {
        Some(owner) => CallableAssistantOwner {
            kind: CallableAssistantOwnerKind::Shared,
            device_name: Some(owner.device_name),
        },
        None => CallableAssistantOwner {
            kind: CallableAssistantOwnerKind::Mine,
            device_name: None,
        },
    };
    Some(CallableAssistant {
        agent_type: info.id,
        name: info.name,
        owner,
        available,
        unavailable_reason,
    })
}

/// What decides which assistants a conversation may call, read from the
/// session rather than from a turn.
struct SessionFacts {
    agent_type: String,
    workspace_id: Option<String>,
    workspace_root: Option<std::path::PathBuf>,
    can_spawn: bool,
    ceiling: openbitfun_runtime_ports::PermissionRuntimeCeiling,
}

impl SessionFacts {
    async fn read(
        coordinator: &crate::agentic::coordination::ConversationCoordinator,
        session_id: &str,
    ) -> OpenBitFunResult<Self> {
        let sessions = coordinator.get_session_manager();
        let session = sessions
            .get_session(session_id)
            .ok_or_else(|| OpenBitFunError::NotFound(format!("Session not found: {session_id}")))?;
        let binding = sessions.resolve_session_workspace_binding(session_id).await;
        let workspace_id = binding
            .as_ref()
            .and_then(|binding| binding.workspace_id.clone());
        let workspace_root = binding
            .as_ref()
            .map(|binding| binding.root_path().to_path_buf());
        let can_spawn = get_agent_registry()
            .get_agent_tools(&session.agent_type, workspace_id.as_deref())
            .await
            .iter()
            .any(|tool| tool == "AgentSpawn" || tool == "Task");
        let ceiling = crate::agentic::permission_policy::load_parent_permission_runtime_ceiling(
            Some(&session.agent_type),
            workspace_id.as_deref(),
        )
        .await?;
        Ok(Self {
            agent_type: session.agent_type,
            workspace_id,
            workspace_root,
            can_spawn,
            ceiling,
        })
    }

    /// Why the conversation's next turn could call no assistant. A turn the
    /// person types is never a review run or a delegated task, so only its
    /// mode, permission restrictions, and swarm membership decide.
    fn refusal(&self) -> Option<CallableAssistantsRefusal> {
        if !self.can_spawn {
            return Some(CallableAssistantsRefusal::Mode);
        }
        let agent_type = self.agent_type.trim();
        parent_refusal(ParentFacts {
            agent_type: Some(agent_type),
            workspace_id: self.workspace_id.as_deref(),
            is_review: agent_type == DEEP_REVIEW_AGENT_TYPE,
            ceiling: &self.ceiling,
            is_swarm: agent_type == "Ultimate" || is_swarm_delegate_agent_type(agent_type),
        })
        .map(Into::into)
    }
}

/// The assistants the person may name in `session_id`: this device's other
/// assistants, then the shared ones that can take work, then those that
/// cannot, each marked so. Lists none, with the reason, when the
/// conversation could not call one.
pub async fn callable_assistants_for_session(
    session_id: &str,
) -> OpenBitFunResult<CallableAssistants> {
    let Some(coordinator) = get_global_coordinator() else {
        return Ok(CallableAssistants::refused(
            CallableAssistantsRefusal::Unavailable,
        ));
    };
    let facts = SessionFacts::read(&coordinator, session_id).await?;
    if let Some(refusal) = facts.refusal() {
        return Ok(CallableAssistants::refused(refusal));
    }
    let assistant_base = crate::infrastructure::try_get_path_manager_arc()
        .ok()
        .map(|path_manager| path_manager.assistant_workspace_base_dir(None));
    let caller = AssistantCaller::of_turn(
        facts.workspace_root.as_deref(),
        None,
        assistant_base.as_deref(),
    );
    if caller.refusal().is_some() {
        return Ok(CallableAssistants::refused(
            CallableAssistantsRefusal::Trust,
        ));
    }
    let local = crate::agentic::agents::local_assistant_entries().await;
    let shared = match coordinator.remote_agent_port() {
        Some(port) => {
            port.list_agents(openbitfun_runtime_ports::RemoteAgentListing::Cached)
                .await
        }
        None => Vec::new(),
    };
    Ok(CallableAssistants::listed(
        crate::agentic::agents::assistant_category(&[], local, shared, &caller),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentic::agents::{assistant_category, LocalAssistantEntry};
    use openbitfun_runtime_ports::{
        RemoteAgentAddress, RemoteAgentDescriptor, RemoteAgentUnavailableReason,
    };

    fn shared(assistant_id: &str, availability: RemoteAgentAvailability) -> RemoteAgentDescriptor {
        RemoteAgentDescriptor {
            address: RemoteAgentAddress {
                device_id: "owner-device".to_string(),
                assistant_id: assistant_id.to_string(),
            },
            agent_type: format!("assistant:{assistant_id}"),
            display_name: assistant_id.to_string(),
            device_name: "Owner's Mac".to_string(),
            description: None,
            availability,
        }
    }

    #[test]
    fn lists_own_assistants_then_shared_ones_marking_those_that_cannot_take_work() {
        let category = assistant_category(
            &[],
            vec![LocalAssistantEntry {
                id: "workspace".to_string(),
                name: "Xiaolan".to_string(),
                description: None,
            }],
            vec![shared(
                "writer",
                RemoteAgentAvailability::Unavailable {
                    reason: RemoteAgentUnavailableReason::Offline,
                    detail: "offline".to_string(),
                },
            )],
            &AssistantCaller::default(),
        );

        let listed = CallableAssistants::listed(category);

        assert!(listed.supported);
        assert_eq!(listed.assistants.len(), 2);
        let mine = &listed.assistants[0];
        assert_eq!(mine.agent_type, "assistant:workspace");
        assert_eq!(mine.owner.kind, CallableAssistantOwnerKind::Mine);
        assert!(mine.available);
        let theirs = &listed.assistants[1];
        assert_eq!(theirs.agent_type, "shared:owner-device/writer");
        assert_eq!(theirs.owner.device_name.as_deref(), Some("Owner's Mac"));
        assert!(!theirs.available);
        assert_eq!(
            theirs.unavailable_reason,
            Some(RemoteAgentUnavailableReason::Offline)
        );
    }

    #[test]
    fn an_assistants_own_conversation_does_not_name_itself() {
        let entry = |id: &str| LocalAssistantEntry {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
        };
        let caller = AssistantCaller {
            own_assistant_id: Some("workspace".to_string()),
            external_trust: None,
        };

        let listed = CallableAssistants::listed(assistant_category(
            &[],
            vec![entry("workspace"), entry("nest")],
            Vec::new(),
            &caller,
        ));

        let names: Vec<_> = listed
            .assistants
            .iter()
            .map(|assistant| assistant.agent_type.as_str())
            .collect();
        assert_eq!(names, ["assistant:nest"]);
    }

    fn facts(agent_type: &str, can_spawn: bool) -> SessionFacts {
        SessionFacts {
            agent_type: agent_type.to_string(),
            workspace_id: None,
            workspace_root: None,
            can_spawn,
            ceiling: Default::default(),
        }
    }

    #[test]
    fn a_conversation_that_cannot_call_subagents_names_no_assistant() {
        assert_eq!(
            facts("Standard", false).refusal(),
            Some(CallableAssistantsRefusal::Mode)
        );
        assert_eq!(
            facts("Ultimate", true).refusal(),
            Some(CallableAssistantsRefusal::Swarm)
        );
        assert_eq!(
            facts(DEEP_REVIEW_AGENT_TYPE, true).refusal(),
            Some(CallableAssistantsRefusal::Review)
        );
        assert_eq!(
            facts("Minimal", true).refusal(),
            Some(CallableAssistantsRefusal::Mode)
        );
        let mut restricted = facts("Standard", true);
        restricted.ceiling = openbitfun_runtime_ports::PermissionRuntimeCeiling::try_new(vec![
            openbitfun_runtime_ports::PermissionRule {
                action: "edit".to_string(),
                resource: "*".to_string(),
                effect: openbitfun_runtime_ports::PermissionEffect::Deny,
            },
        ])
        .expect("deny-only ceiling");
        assert_eq!(
            restricted.refusal(),
            Some(CallableAssistantsRefusal::PermissionRestrictions)
        );
        assert_eq!(facts("Standard", true).refusal(), None);
    }

    #[test]
    fn the_response_keeps_its_wire_shape() {
        let refused =
            CallableAssistants::refused(CallableAssistantsRefusal::PermissionRestrictions);
        assert_eq!(
            serde_json::to_value(&refused).expect("serializes"),
            serde_json::json!({
                "supported": false,
                "reason": "permissionRestrictions",
                "assistants": [],
            })
        );
    }
}
