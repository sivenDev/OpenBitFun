//! Calls a task another device delegated makes to assistants.
//!
//! At full trust such a task may call this device's other assistants and the
//! assistants shared with this device. Each call is that same device's work:
//! it counts against the share's concurrent tasks, is recorded in the audit
//! log under that device, and ends when the share's access does. The host
//! that runs delegated tasks keeps that account; the kernel asks it before a
//! call starts and holds the ticket it returns until the call ends.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::service::delegation::DelegatedTurnFacts;

/// A call a delegated task asks to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedCall {
    /// The called agent: `assistant:<id>` or `shared:<device>/<assistant>`.
    pub agent_type: String,
    /// For a call to this device's assistant, its id.
    pub local_assistant_id: Option<String>,
}

/// The account of one call; the call holds it until it ends, and dropping it
/// gives the place back.
pub struct DelegatedCallTicket {
    /// For a call to this device's assistant, the workspace the calling
    /// device has under it, where the call's session is kept.
    pub tenant_workspace: Option<PathBuf>,
    hold: Box<dyn DelegatedCallHold>,
}

impl DelegatedCallTicket {
    pub fn new(tenant_workspace: Option<PathBuf>, hold: Box<dyn DelegatedCallHold>) -> Self {
        Self {
            tenant_workspace,
            hold,
        }
    }

    /// Records the session the call runs in, titled `title`, so it is
    /// listed, kept and removed with the delegated task's own.
    pub fn record_session(&self, workspace_path: &str, session_id: &str, title: &str) {
        self.hold.record_session(workspace_path, session_id, title);
    }
}

impl std::fmt::Debug for DelegatedCallTicket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DelegatedCallTicket")
            .field("tenant_workspace", &self.tenant_workspace)
            .finish_non_exhaustive()
    }
}

/// What a ticket holds on the host; dropping it releases the place.
pub trait DelegatedCallHold: Send + Sync {
    fn record_session(&self, workspace_path: &str, session_id: &str, title: &str);
}

/// The host's account of delegated tasks' calls.
#[async_trait::async_trait]
pub trait DelegatedCallLedger: Send + Sync {
    /// Admits `call` for the delegated task this process runs, or says why
    /// not: access withdrawn, trust lowered, or the share already runs as
    /// many tasks as it allows.
    async fn open(&self, call: &DelegatedCall) -> Result<DelegatedCallTicket, String>;

    /// This device's sharing id, which the task's call chain names this
    /// device by, so a call that comes back across devices is seen as a loop.
    fn local_device_id(&self) -> Option<String> {
        None
    }
}

/// The call a turn with `context` is asking to start, when the turn is a
/// delegated task's and the agent is an assistant.
pub(crate) fn delegated_call(
    context: &HashMap<String, String>,
    agent_type: &str,
) -> Option<DelegatedCall> {
    DelegatedTurnFacts::from_context(context)?;
    if let Some(id) = crate::service::delegation::assistant_directory(agent_type) {
        return Some(DelegatedCall {
            agent_type: agent_type.to_string(),
            local_assistant_id: Some(id.to_string()),
        });
    }
    super::remote_agent_turn::remote_agent_address(agent_type).map(|_| DelegatedCall {
        agent_type: agent_type.to_string(),
        local_assistant_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::delegation::{
        DELEGATION_TRUST_CONTEXT_KEY, DISPATCH_DELEGATION_CONTEXT_KEY,
    };

    #[test]
    fn only_a_delegated_tasks_calls_to_assistants_are_accounted() {
        let delegated = HashMap::from([
            (
                DISPATCH_DELEGATION_CONTEXT_KEY.to_string(),
                "true".to_string(),
            ),
            (DELEGATION_TRUST_CONTEXT_KEY.to_string(), "full".to_string()),
        ]);
        assert_eq!(
            delegated_call(&delegated, "assistant:workspace-notes"),
            Some(DelegatedCall {
                agent_type: "assistant:workspace-notes".to_string(),
                local_assistant_id: Some("workspace-notes".to_string()),
            })
        );
        assert_eq!(
            delegated_call(&delegated, "shared:laptop/writer"),
            Some(DelegatedCall {
                agent_type: "shared:laptop/writer".to_string(),
                local_assistant_id: None,
            })
        );
        assert_eq!(delegated_call(&delegated, "Explore"), None);
        assert_eq!(
            delegated_call(&HashMap::new(), "assistant:workspace-notes"),
            None
        );
    }
}
