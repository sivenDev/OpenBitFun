use std::collections::HashMap;

use super::*;
use crate::service::delegation::{
    DELEGATION_ASSISTANT_CONTEXT_KEY, DELEGATION_TENANT_CONTEXT_KEY, DELEGATION_TRUST_CONTEXT_KEY,
};

fn call() -> HashMap<String, String> {
    HashMap::from([
        (
            DELEGATION_CALL_CHAIN_CONTEXT_KEY.to_string(),
            r#"["device/workspace","device/workspace-x"]"#.to_string(),
        ),
        (
            ASSISTANT_CALL_CALLER_CONTEXT_KEY.to_string(),
            "assistant:workspace-x".to_string(),
        ),
        (
            ASSISTANT_CALL_CALLER_NAME_CONTEXT_KEY.to_string(),
            "X".to_string(),
        ),
        ("permission_mode".to_string(), "default".to_string()),
    ])
}

#[test]
fn the_session_records_its_caller_and_the_chain_it_came_through() {
    let delegated = delegated_by(&call(), "caller-session");
    assert_eq!(delegated.assistant, "assistant:workspace-x");
    assert_eq!(delegated.assistant_name.as_deref(), Some("X"));
    assert_eq!(delegated.session_id, "caller-session");
    assert_eq!(
        delegated.call_chain,
        vec!["device/workspace", "device/workspace-x"]
    );
}

#[test]
fn the_called_assistant_runs_as_the_owner_task_with_the_chain_only() {
    let mut call = call();
    for key in [
        DISPATCH_DELEGATION_CONTEXT_KEY,
        DELEGATION_ASSISTANT_CONTEXT_KEY,
        DELEGATION_TENANT_CONTEXT_KEY,
        DELEGATION_TRUST_CONTEXT_KEY,
    ] {
        call.insert(key.to_string(), "stale".to_string());
    }
    let context = child_context(&call);
    assert!(DelegatedTurnFacts::from_context(&context).is_none());
    for key in [
        DISPATCH_DELEGATION_CONTEXT_KEY,
        DELEGATION_ASSISTANT_CONTEXT_KEY,
        DELEGATION_TENANT_CONTEXT_KEY,
        DELEGATION_TRUST_CONTEXT_KEY,
        ASSISTANT_CALL_CALLER_CONTEXT_KEY,
        ASSISTANT_CALL_CALLER_NAME_CONTEXT_KEY,
    ] {
        assert!(!context.contains_key(key), "{key} carried over");
    }
    assert_eq!(
        call_chain(&context),
        vec!["device/workspace", "device/workspace-x"]
    );
    assert_eq!(context["permission_mode"], "default");
}

#[test]
fn the_session_is_named_after_the_task_and_is_not_a_hidden_subagent() {
    assert_eq!(
        session_name("\n  Draft the notes  \nwith care"),
        "Draft the notes"
    );
    assert_eq!(session_name(&"a".repeat(200)).chars().count(), 80);
    assert!(!created_by("caller-session").starts_with("session-"));
}

#[test]
fn the_called_assistant_is_told_which_assistant_is_asking() {
    let reminder = call_reminder(&call());
    assert!(reminder.contains("\"X\", another assistant of the person you work for"));
    assert!(reminder.contains("Answer as yourself"));
    let unnamed = call_reminder(&HashMap::new());
    assert!(unnamed.starts_with("The next message comes from Another assistant"));
}

#[test]
fn a_delegated_tasks_call_stays_that_devices_task_under_the_called_assistant() {
    let mut call = call();
    call.insert(
        DISPATCH_DELEGATION_CONTEXT_KEY.to_string(),
        "true".to_string(),
    );
    call.insert(DELEGATION_TRUST_CONTEXT_KEY.to_string(), "full".to_string());
    call.insert(
        DELEGATION_ASSISTANT_CONTEXT_KEY.to_string(),
        "/assistants/workspace-x".to_string(),
    );
    call.insert(
        DELEGATION_TENANT_CONTEXT_KEY.to_string(),
        "/tenants/workspace-x/device-b".to_string(),
    );
    call.insert(
        ASSISTANT_CALL_TENANT_WORKSPACE_CONTEXT_KEY.to_string(),
        "/tenants/workspace/device-b".to_string(),
    );

    let context = tenant_child_context(
        &call,
        std::path::Path::new("/assistants/workspace"),
        std::path::Path::new("/tenants/workspace/device-b"),
    );
    let facts = DelegatedTurnFacts::from_context(&context).expect("still a delegated task");
    assert_eq!(
        facts.assistant_workspace.as_deref(),
        Some(std::path::Path::new("/assistants/workspace"))
    );
    assert_eq!(
        facts.tenant_workspace.as_deref(),
        Some(std::path::Path::new("/tenants/workspace/device-b"))
    );
    assert_eq!(
        facts.trust,
        crate::service::delegation::DelegationTrust::Full
    );
    assert_eq!(
        call_chain(&context),
        vec!["device/workspace", "device/workspace-x"]
    );
    for key in [
        ASSISTANT_CALL_CALLER_CONTEXT_KEY,
        ASSISTANT_CALL_CALLER_NAME_CONTEXT_KEY,
        ASSISTANT_CALL_TENANT_WORKSPACE_CONTEXT_KEY,
    ] {
        assert!(!context.contains_key(key), "{key} carried over");
    }

    // It is told the work is for a person on another device.
    let reminder = call_reminder(&call);
    assert!(reminder.contains("a person on another device delegated"));
    assert!(reminder.contains("same sharing rules"));
    // An owner's own call does not keep the workspace it was not given.
    assert!(!child_context(&call).contains_key(ASSISTANT_CALL_TENANT_WORKSPACE_CONTEXT_KEY));
}
