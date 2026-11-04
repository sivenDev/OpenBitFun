//! A call from one of this device's assistants to another of them.
//!
//! The called assistant works as itself, in its own default workspace, in a
//! session of its own: listed with its other sessions and recording its
//! caller, not a hidden child of the calling session. The result reaches the
//! caller as any background subagent's does, through `AgentWait`.

use std::collections::HashMap;

use openbitfun_services_core::session::SessionDelegatedBy;

use crate::service::delegation::{
    DelegatedTurnFacts, DELEGATION_CALL_CHAIN_CONTEXT_KEY, DISPATCH_DELEGATION_CONTEXT_KEY,
};

/// The calling assistant's agent type, `assistant:<directory>`, in the
/// context of a call to another assistant.
pub(crate) const ASSISTANT_CALL_CALLER_CONTEXT_KEY: &str = "assistantCallCaller";
/// The calling assistant's name when the call was made.
pub(crate) const ASSISTANT_CALL_CALLER_NAME_CONTEXT_KEY: &str = "assistantCallCallerName";

const SESSION_NAME_MAX_CHARS: usize = 80;

/// The creator recorded on a session an assistant call started. It does not
/// start with `session-`, which marks hidden subagent sessions.
pub(crate) fn created_by(parent_session_id: &str) -> String {
    format!("assistant-call-{parent_session_id}")
}

/// The name of the called assistant's session: the task's first line.
pub(crate) fn session_name(task: &str) -> String {
    task.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(SESSION_NAME_MAX_CHARS)
        .collect()
}

/// The call chain the caller put in the call's context.
pub(crate) fn call_chain(context: &HashMap<String, String>) -> Vec<String> {
    context
        .get(DELEGATION_CALL_CHAIN_CONTEXT_KEY)
        .and_then(|chain| serde_json::from_str(chain).ok())
        .unwrap_or_default()
}

/// Who started the called assistant's session, from the call's context.
pub(crate) fn delegated_by(
    context: &HashMap<String, String>,
    parent_session_id: &str,
) -> SessionDelegatedBy {
    SessionDelegatedBy {
        assistant: context
            .get(ASSISTANT_CALL_CALLER_CONTEXT_KEY)
            .cloned()
            .unwrap_or_default(),
        assistant_name: context
            .get(ASSISTANT_CALL_CALLER_NAME_CONTEXT_KEY)
            .filter(|name| !name.trim().is_empty())
            .cloned(),
        session_id: parent_session_id.to_string(),
        call_chain: call_chain(context),
    }
}

/// What the called assistant is told about the call: who is asking and on
/// whose behalf, so it answers as itself to another assistant of the same
/// person rather than as if that person were typing.
pub(crate) fn call_reminder(context: &HashMap<String, String>) -> String {
    let caller = context
        .get(ASSISTANT_CALL_CALLER_NAME_CONTEXT_KEY)
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| format!("\"{name}\""))
        .unwrap_or_else(|| "Another assistant".to_string());
    format!(
        "The next message comes from {caller}, another assistant of the person you work for, on this same device. \
         It is calling you for help with that person's work. Answer as yourself; your reply goes back to that assistant, \
         which passes on what matters."
    )
}

/// The context of the called assistant's turn. It is the owner's own task,
/// not one another device delegated, so no delegation fact of the caller
/// carries over; the call chain does, so a call back to an assistant on it
/// is refused.
pub(crate) fn child_context(call: &HashMap<String, String>) -> HashMap<String, String> {
    let mut context = call.clone();
    context.remove(DISPATCH_DELEGATION_CONTEXT_KEY);
    for key in DelegatedTurnFacts::INHERITED_CONTEXT_KEYS {
        if key != DELEGATION_CALL_CHAIN_CONTEXT_KEY {
            context.remove(key);
        }
    }
    context.remove(ASSISTANT_CALL_CALLER_CONTEXT_KEY);
    context.remove(ASSISTANT_CALL_CALLER_NAME_CONTEXT_KEY);
    context
}

#[cfg(test)]
mod tests;
