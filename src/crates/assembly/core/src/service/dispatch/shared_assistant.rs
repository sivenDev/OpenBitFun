//! Conversations with an assistant another device shares on the local
//! network.
//!
//! Each conversation is a delegated job on the owner's device: it runs as the
//! shared assistant in this device's workspace there, with the owner's model
//! and the owner's approvals. Nothing is delivered back through Git, so the
//! outbound record carries no workspace. Follow-ups, appended messages,
//! status and cancellation take the device paths through `DeviceDispatchRpc`;
//! this module adds the submission, the listing by assistant, and the reasons
//! a conversation cannot go on ([`SharedAssistantProblem`]).

use anyhow::anyhow;
use openbitfun_services_core::dispatch_contract::{
    DispatchCatalogResponse, DISPATCH_AGENT_DELEGATION_CAPABILITY,
};
use openbitfun_services_integrations::remote_ssh::dispatch_ssh;
use serde_json::{json, Value};

use crate::service::delegation::DELEGATED_JOB_ID_PREFIX;

use super::controller::{
    validate_device_attachment_budget, validate_submit_ack, validate_submit_request,
    DispatchAppendRequest, DispatchContinueRequest, DispatchJobRequest, DispatchListJobsRequest,
    DispatchQueryJobRequest, DispatchStatusRequest, DispatchSubmitRequest,
    DISPATCH_PROTOCOL_VERSION,
};
use super::device_controller::{
    append_device, cancel_device, continue_device_job, query_device_job, status_device,
    DeviceDispatchRpc,
};
use super::target::same_shared_assistant;
use super::{DispatchTarget, DispatchTargetRequest, OutboundDispatchRecord, OutboundDispatchStore};

/// The only approval policy of a shared assistant's tasks: its owner answers.
const SHARED_ASSISTANT_APPROVAL_POLICY: &str = "remote";

/// Why a conversation with a shared assistant cannot go on. The message
/// starts with `[shared-assistant:<code>]` so every surface can tell the
/// reasons apart without parsing the rest.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SharedAssistantProblem {
    /// This device no longer keeps the assistant: it was removed here.
    #[error("[shared-assistant:notShared] this assistant is not shared with this device")]
    NotShared,
    /// The owner withdrew access or stopped sharing.
    #[error("[shared-assistant:revoked] the owner withdrew access to this assistant")]
    Revoked,
    /// The owner's device cannot be reached on the local network.
    #[error("[shared-assistant:offline] the owner's device is not reachable: {0}")]
    Offline(String),
    /// One of the two devices runs a version the other cannot talk to.
    #[error("[shared-assistant:needsUpdate] {0}")]
    NeedsUpdate(String),
    /// The owner no longer keeps the conversation: it outlived the retention
    /// period or its data was deleted.
    #[error("[shared-assistant:expired] the owner no longer keeps this conversation")]
    Expired,
    /// The assistant is shared but cannot take tasks, for the reason given.
    #[error("[shared-assistant:unavailable] {0}")]
    Unavailable(String),
}

impl SharedAssistantProblem {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotShared => "notShared",
            Self::Revoked => "revoked",
            Self::Offline(_) => "offline",
            Self::NeedsUpdate(_) => "needsUpdate",
            Self::Expired => "expired",
            Self::Unavailable(_) => "unavailable",
        }
    }
}

/// Names the reason a call failed when the owner's answer says it. `job`
/// is set for calls about a conversation this device has a record of.
fn explain(error: anyhow::Error, job: bool) -> anyhow::Error {
    if error.downcast_ref::<SharedAssistantProblem>().is_some() {
        return error;
    }
    let text = format!("{error:#}");
    // The owner's runner answers the same for a job it removed and for one
    // that never belonged to this device; a recorded one was removed.
    if job && text.contains("dispatch job '") && text.contains("' does not exist") {
        return SharedAssistantProblem::Expired.into();
    }
    // Refusals of the owner's share store (`ShareRefusal`).
    if [
        "is not shared on this device",
        "this device has no access to assistant",
        "the owner declined further requests from this device",
    ]
    .iter()
    .any(|refusal| text.contains(refusal))
    {
        return SharedAssistantProblem::Revoked.into();
    }
    error
}

fn shared_assistant_of(target: &DispatchTargetRequest) -> anyhow::Result<(&str, &str)> {
    let DispatchTargetRequest::SharedAssistant {
        owner_device_id,
        assistant_id,
    } = target
    else {
        anyhow::bail!("this call requires a shared assistant target");
    };
    if owner_device_id.trim().is_empty() || assistant_id.trim().is_empty() {
        anyhow::bail!("a shared assistant target names its owner's device and the assistant");
    }
    Ok((owner_device_id, assistant_id))
}

/// Starts a conversation: a delegated job on the owner's device, approved by
/// its owner and run with the owner's model. The assistant runs as the agent
/// the owner's catalog names, whatever `agentType` the request carries.
pub async fn submit_shared_assistant(
    rpc: &dyn DeviceDispatchRpc,
    store: &OutboundDispatchStore,
    request: DispatchSubmitRequest,
) -> anyhow::Result<Value> {
    validate_submit_request(&request)?;
    let (owner, assistant_id) = shared_assistant_of(&request.target)?;
    if !request.job_id.starts_with(DELEGATED_JOB_ID_PREFIX) {
        anyhow::bail!(
            "conversations with shared assistants use '{DELEGATED_JOB_ID_PREFIX}' job ids"
        );
    }
    if request.approval_policy != SHARED_ASSISTANT_APPROVAL_POLICY {
        anyhow::bail!("tasks for shared assistants are approved by their owner");
    }
    if request.base_ref.is_some() || request.include_uncommitted {
        anyhow::bail!("shared assistants do not take a Git baseline");
    }
    validate_device_attachment_budget(&request.attachments)?;

    let protocol = rpc
        .invoke(owner, "dispatch_target_probe", json!({}))
        .await
        .map_err(|error| explain(error, false))?;
    dispatch_ssh::validate_dispatch_protocol(&protocol, Some(SHARED_ASSISTANT_APPROVAL_POLICY))
        .map_err(|error| SharedAssistantProblem::NeedsUpdate(format!("{error:#}")))?;
    let delegates = protocol
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .any(|capability| capability.as_str() == Some(DISPATCH_AGENT_DELEGATION_CAPABILITY))
        });
    if !delegates {
        return Err(SharedAssistantProblem::NeedsUpdate(
            "the owner's device does not take tasks for shared assistants".to_string(),
        )
        .into());
    }
    let catalog: DispatchCatalogResponse = serde_json::from_value(
        rpc.invoke(owner, "dispatch_target_catalog", json!({}))
            .await
            .map_err(|error| explain(error, false))?,
    )
    .map_err(|error| anyhow!("decode the shared assistants of the owner's device: {error}"))?;
    let agent = catalog
        .agents
        .into_iter()
        .find(|agent| agent.id == assistant_id)
        .ok_or(SharedAssistantProblem::Revoked)?;
    if let Some(reason) = agent.unavailable {
        return Err(SharedAssistantProblem::Unavailable(reason).into());
    }

    let target = DispatchTarget::SharedAssistant {
        owner_device_id: owner.to_string(),
        assistant_id: assistant_id.to_string(),
        display_name: agent.display_name,
    };
    let requested = OutboundDispatchRecord::new(
        request.job_id.clone(),
        target,
        request.session_id.clone(),
        String::new(),
        &request.prompt,
        "submitting",
    )?
    .with_submission_metadata(
        request.title.clone(),
        agent.agent_type.clone(),
        SHARED_ASSISTANT_APPROVAL_POLICY.to_string(),
        None,
        None,
    );
    let bound = store.bind_if_absent(&requested).await?;
    if bound.session_id != requested.session_id
        || !same_shared_assistant(&bound.target, &requested.target)
    {
        anyhow::bail!("Dispatch jobId is already bound to another target or session");
    }

    let mut payload = json!({
        "protocolVersion": DISPATCH_PROTOCOL_VERSION,
        "jobId": request.job_id,
        "sessionId": request.session_id,
        "workspacePath": "",
        "agentType": agent.agent_type,
        "prompt": request.prompt,
        "approvalPolicy": SHARED_ASSISTANT_APPROVAL_POLICY,
        "delegation": { "assistantId": assistant_id },
    });
    if let Some(title) = request.title.filter(|value| !value.trim().is_empty()) {
        payload["title"] = Value::String(title);
    }
    if !request.attachments.is_empty() {
        payload["attachments"] = serde_json::to_value(&request.attachments)?;
    }
    let response = match rpc.invoke(owner, "dispatch_target_submit", payload).await {
        Ok(response) => response,
        Err(error) => {
            let _ = store
                .update_progress(&requested.job_id, 0, "submission_unknown")
                .await;
            return Err(explain(error, false));
        }
    };
    if let Err(error) = validate_submit_ack(&response, &requested.job_id, &requested.session_id) {
        let _ = store
            .update_progress(&requested.job_id, 0, "submission_unknown")
            .await;
        return Err(error);
    }
    let state = response
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("queued")
        .to_string();
    store.update_progress(&requested.job_id, 0, state).await?;
    Ok(response)
}

/// The conversation's events from `cursor`. A conversation the owner no
/// longer keeps is recorded as `expired`.
pub async fn status_shared_assistant(
    rpc: &dyn DeviceDispatchRpc,
    store: &OutboundDispatchStore,
    request: DispatchStatusRequest,
) -> anyhow::Result<Value> {
    let job_id = request.job_id.clone();
    let cursor = request.cursor;
    match status_device(rpc, store, request).await {
        Ok(response) => Ok(response),
        Err(error) => {
            let error = explain(error, true);
            if matches!(
                error.downcast_ref::<SharedAssistantProblem>(),
                Some(SharedAssistantProblem::Expired)
            ) {
                if let Err(record_error) = store.update_progress(&job_id, cursor, "expired").await {
                    log::warn!(
                        "Failed to record an expired shared assistant conversation: job_id={} error={}",
                        job_id,
                        record_error
                    );
                }
            }
            Err(error)
        }
    }
}

/// The next turn of a conversation. The owner chooses the model and answers
/// the approvals, so the request's choices of either are dropped.
pub async fn continue_shared_assistant(
    rpc: &dyn DeviceDispatchRpc,
    store: &OutboundDispatchStore,
    mut request: DispatchContinueRequest,
) -> anyhow::Result<Value> {
    request.model = None;
    request.reasoning_preset = None;
    request.approval_policy = None;
    continue_device_job(rpc, store, request)
        .await
        .map_err(|error| explain(error, true))
}

/// A message for the turn that is running.
pub async fn append_shared_assistant(
    rpc: &dyn DeviceDispatchRpc,
    store: &OutboundDispatchStore,
    request: DispatchAppendRequest,
) -> anyhow::Result<Value> {
    append_device(rpc, store, request)
        .await
        .map_err(|error| explain(error, true))
}

pub async fn cancel_shared_assistant(
    rpc: &dyn DeviceDispatchRpc,
    store: &OutboundDispatchStore,
    request: DispatchJobRequest,
) -> anyhow::Result<Value> {
    cancel_device(rpc, store, request)
        .await
        .map_err(|error| explain(error, true))
}

/// Reports about a conversation; the owner answers only `usageReport`.
pub async fn query_shared_assistant(
    rpc: &dyn DeviceDispatchRpc,
    store: &OutboundDispatchStore,
    request: DispatchQueryJobRequest,
) -> anyhow::Result<Value> {
    query_device_job(rpc, store, request)
        .await
        .map_err(|error| explain(error, true))
}

/// This device's conversations with one shared assistant, newest first as
/// the owner lists them; each gets an outbound record here so it can be
/// opened again. `display_name` names the assistant in new records.
pub async fn list_shared_assistant_jobs(
    rpc: &dyn DeviceDispatchRpc,
    store: &OutboundDispatchStore,
    display_name: String,
    request: DispatchListJobsRequest,
) -> anyhow::Result<Value> {
    let target = request
        .target
        .as_ref()
        .ok_or_else(|| anyhow!("listing a shared assistant's conversations names the assistant"))?;
    let (owner, assistant_id) = shared_assistant_of(target)?;
    let listed = rpc
        .invoke(owner, "dispatch_target_list", json!({}))
        .await
        .map_err(|error| explain(error, false))?;
    let entries = listed
        .as_array()
        .ok_or_else(|| anyhow!("the owner's device returned an invalid conversation list"))?;
    let mut conversations = Vec::new();
    for entry in entries {
        if entry
            .pointer("/delegation/assistantId")
            .and_then(Value::as_str)
            != Some(assistant_id)
        {
            continue;
        }
        let text = |field: &str| entry.get(field).and_then(Value::as_str).unwrap_or_default();
        let (job_id, session_id, state) = (text("jobId"), text("sessionId"), text("state"));
        if !job_id.starts_with(DELEGATED_JOB_ID_PREFIX)
            || !matches!(
                state,
                "queued" | "running" | "succeeded" | "failed" | "cancelled"
            )
        {
            anyhow::bail!("the owner's device returned an invalid conversation");
        }
        openbitfun_agent_runtime::session_control::validate_session_id(session_id)
            .map_err(anyhow::Error::msg)?;
        let target = DispatchTarget::SharedAssistant {
            owner_device_id: owner.to_string(),
            assistant_id: assistant_id.to_string(),
            display_name: display_name.clone(),
        };
        let title = text("title");
        let mut requested = OutboundDispatchRecord::new(
            job_id.to_string(),
            target,
            session_id.to_string(),
            String::new(),
            title,
            state,
        )?;
        requested.title = (!title.trim().is_empty()).then(|| title.to_string());
        requested.agent_type =
            Some(text("agentType").to_string()).filter(|value| !value.is_empty());
        requested.approval_policy = Some(SHARED_ASSISTANT_APPROVAL_POLICY.to_string());
        if let Some(started_at) = entry
            .get("startedAt")
            .and_then(Value::as_str)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        {
            requested.created_at = started_at.with_timezone(&chrono::Utc);
            requested.updated_at = requested.created_at;
        }
        let bound = store.bind_if_absent(&requested).await?;
        if bound.session_id != requested.session_id
            || !same_shared_assistant(&bound.target, &requested.target)
        {
            anyhow::bail!(
                "dispatch jobId is already bound to another target or session on this controller"
            );
        }
        if bound.last_state != state && bound.last_state != "expired" {
            store
                .update_progress(job_id, bound.last_cursor, state)
                .await?;
        }
        conversations.push(entry.clone());
    }
    Ok(Value::Array(conversations))
}

/// Deletes this device's records and transcripts of its conversations with
/// a shared assistant it removed. Returns how many conversations were
/// deleted.
pub async fn forget_shared_assistant(
    store: &OutboundDispatchStore,
    owner_device_id: &str,
    assistant_id: &str,
) -> anyhow::Result<usize> {
    let removed = DispatchTarget::SharedAssistant {
        owner_device_id: owner_device_id.to_string(),
        assistant_id: assistant_id.to_string(),
        display_name: String::new(),
    };
    let mut count = 0;
    for record in store.list().await? {
        if !same_shared_assistant(&record.target, &removed) {
            continue;
        }
        store.remove_transcript(&record.job_id).await?;
        if store.remove(&record.job_id).await? {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    const OWNER: &str = "0123456789abcdef0123456789abcdef";
    const JOB: &str = "delegation-1";
    const SESSION: &str = "11111111-1111-4111-8111-111111111111";

    /// The owner's device: answers each command from `answers`, in order of
    /// appearance, and records what it received.
    struct Owner {
        answers: Mutex<Vec<(&'static str, anyhow::Result<Value>)>>,
        received: Mutex<Vec<(String, Value)>>,
    }

    impl Owner {
        fn new(answers: Vec<(&'static str, anyhow::Result<Value>)>) -> Self {
            Self {
                answers: Mutex::new(answers),
                received: Mutex::new(Vec::new()),
            }
        }

        fn received(&self, command: &str) -> Value {
            self.received
                .lock()
                .unwrap()
                .iter()
                .find(|(name, _)| name == command)
                .map(|(_, args)| args.clone())
                .unwrap_or_else(|| panic!("{command} was not sent"))
        }
    }

    #[async_trait]
    impl DeviceDispatchRpc for Owner {
        async fn invoke(
            &self,
            device_id: &str,
            command: &str,
            args: Value,
        ) -> anyhow::Result<Value> {
            assert_eq!(device_id, OWNER);
            self.received
                .lock()
                .unwrap()
                .push((command.to_string(), args));
            let mut answers = self.answers.lock().unwrap();
            let index = answers
                .iter()
                .position(|(name, _)| *name == command)
                .unwrap_or_else(|| panic!("unexpected {command}"));
            answers.remove(index).1
        }
    }

    fn protocol(delegates: bool) -> Value {
        let mut capabilities: Vec<_> =
            openbitfun_services_core::dispatch_contract::dispatch_required_target_capabilities()
                .collect();
        if delegates {
            capabilities.push(DISPATCH_AGENT_DELEGATION_CAPABILITY);
        }
        json!({
            "productId": openbitfun_services_core::product_identity::product_id(),
            "dataNamespace": openbitfun_services_core::product_identity::data_namespace(),
            "protocolVersion": DISPATCH_PROTOCOL_VERSION,
            "capabilities": capabilities,
        })
    }

    fn catalog(unavailable: Option<&str>) -> Value {
        json!({ "agents": [{
            "id": "workspace-notes",
            "agentType": "assistant:workspace-notes",
            "displayName": "Notes",
            "unavailable": unavailable,
        }] })
    }

    fn target() -> DispatchTargetRequest {
        DispatchTargetRequest::SharedAssistant {
            owner_device_id: OWNER.to_string(),
            assistant_id: "workspace-notes".to_string(),
        }
    }

    fn submission() -> DispatchSubmitRequest {
        serde_json::from_value(json!({
            "target": { "kind": "sharedAssistant", "ownerDeviceId": OWNER, "assistantId": "workspace-notes" },
            "jobId": JOB,
            "sessionId": SESSION,
            "agentType": "anything",
            "prompt": "Summarize the notes",
            "approvalPolicy": "remote",
            "model": "my-model",
            "title": "Notes summary",
        }))
        .expect("submission")
    }

    fn accepted() -> anyhow::Result<Value> {
        Ok(json!({ "accepted": true, "jobId": JOB, "sessionId": SESSION, "state": "queued" }))
    }

    async fn submitted(dir: &std::path::Path) -> OutboundDispatchStore {
        let store = OutboundDispatchStore::new_in_root_for_tests(dir.to_path_buf());
        let owner = Owner::new(vec![
            ("dispatch_target_probe", Ok(protocol(true))),
            ("dispatch_target_catalog", Ok(catalog(None))),
            ("dispatch_target_submit", accepted()),
        ]);
        submit_shared_assistant(&owner, &store, submission())
            .await
            .expect("submit");
        store
    }

    fn problem(error: &anyhow::Error) -> Option<&SharedAssistantProblem> {
        error.downcast_ref::<SharedAssistantProblem>()
    }

    #[tokio::test]
    async fn a_conversation_is_a_delegated_job_the_owner_approves_with_its_own_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutboundDispatchStore::new_in_root_for_tests(dir.path().to_path_buf());
        let owner = Owner::new(vec![
            ("dispatch_target_probe", Ok(protocol(true))),
            ("dispatch_target_catalog", Ok(catalog(None))),
            ("dispatch_target_submit", accepted()),
        ]);
        let answer = submit_shared_assistant(&owner, &store, submission())
            .await
            .unwrap();
        assert_eq!(answer["state"], "queued");

        let sent = owner.received("dispatch_target_submit");
        assert_eq!(sent["agentType"], "assistant:workspace-notes");
        assert_eq!(sent["approvalPolicy"], "remote");
        assert_eq!(sent["workspacePath"], "");
        assert_eq!(sent["delegation"]["assistantId"], "workspace-notes");
        assert_eq!(sent["title"], "Notes summary");
        assert!(sent.get("model").is_none());
        assert_eq!(owner.received("dispatch_target_probe"), json!({}));

        let record = store.get(JOB).await.unwrap().unwrap();
        assert_eq!(
            record.target,
            DispatchTarget::SharedAssistant {
                owner_device_id: OWNER.to_string(),
                assistant_id: "workspace-notes".to_string(),
                display_name: "Notes".to_string(),
            }
        );
        assert_eq!(record.workspace_path, "");
        assert_eq!(record.source_workspace_path, None);
        assert_eq!(record.model, None);
        assert_eq!(record.last_state, "queued");
        assert_eq!(record.baseline_worktree_id, None);
    }

    #[tokio::test]
    async fn a_submission_is_refused_before_anything_is_sent_when_it_cannot_be_a_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutboundDispatchStore::new_in_root_for_tests(dir.path().to_path_buf());
        let owner = Owner::new(Vec::new());
        let mut changes: Vec<Box<dyn Fn(&mut DispatchSubmitRequest)>> = vec![
            Box::new(|request| request.job_id = "job-1".to_string()),
            Box::new(|request| request.approval_policy = "auto".to_string()),
            Box::new(|request| request.include_uncommitted = true),
            Box::new(|request| request.base_ref = Some("main".to_string())),
            Box::new(|request| {
                request.target = DispatchTargetRequest::SharedAssistant {
                    owner_device_id: OWNER.to_string(),
                    assistant_id: " ".to_string(),
                }
            }),
        ];
        for change in changes.drain(..) {
            let mut request = submission();
            change(&mut request);
            assert!(submit_shared_assistant(&owner, &store, request)
                .await
                .is_err());
        }
        assert!(owner.received.lock().unwrap().is_empty());
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_assistant_that_cannot_take_tasks_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutboundDispatchStore::new_in_root_for_tests(dir.path().to_path_buf());

        let outdated = Owner::new(vec![("dispatch_target_probe", Ok(protocol(false)))]);
        let error = submit_shared_assistant(&outdated, &store, submission())
            .await
            .unwrap_err();
        assert_eq!(problem(&error).map(|p| p.code()), Some("needsUpdate"));

        let gone = Owner::new(vec![
            ("dispatch_target_probe", Ok(protocol(true))),
            (
                "dispatch_target_catalog",
                Ok(catalog(Some(
                    "the assistant no longer exists on this device",
                ))),
            ),
        ]);
        let error = submit_shared_assistant(&gone, &store, submission())
            .await
            .unwrap_err();
        assert_eq!(problem(&error).map(|p| p.code()), Some("unavailable"));
        assert!(error
            .to_string()
            .starts_with("[shared-assistant:unavailable]"));

        let withdrawn = Owner::new(vec![
            ("dispatch_target_probe", Ok(protocol(true))),
            ("dispatch_target_catalog", Ok(json!({ "agents": [] }))),
        ]);
        let error = submit_shared_assistant(&withdrawn, &store, submission())
            .await
            .unwrap_err();
        assert_eq!(problem(&error), Some(&SharedAssistantProblem::Revoked));

        let offline = Owner::new(vec![(
            "dispatch_target_probe",
            Err(SharedAssistantProblem::Offline("connection refused".to_string()).into()),
        )]);
        let error = submit_shared_assistant(&offline, &store, submission())
            .await
            .unwrap_err();
        assert_eq!(problem(&error).map(|p| p.code()), Some("offline"));
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unanswered_submission_is_left_for_the_status_to_settle() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutboundDispatchStore::new_in_root_for_tests(dir.path().to_path_buf());
        let owner = Owner::new(vec![
            ("dispatch_target_probe", Ok(protocol(true))),
            ("dispatch_target_catalog", Ok(catalog(None))),
            (
                "dispatch_target_submit",
                Err(SharedAssistantProblem::Offline("timed out".to_string()).into()),
            ),
        ]);
        let error = submit_shared_assistant(&owner, &store, submission())
            .await
            .unwrap_err();
        assert_eq!(problem(&error).map(|p| p.code()), Some("offline"));
        assert_eq!(
            store.get(JOB).await.unwrap().unwrap().last_state,
            "submission_unknown"
        );
    }

    #[tokio::test]
    async fn follow_ups_appends_status_and_cancel_go_to_the_owner_of_the_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let store = submitted(dir.path()).await;
        let owner = Owner::new(vec![
            (
                "dispatch_target_status",
                Ok(json!({ "jobId": JOB, "state": "running", "events": [], "cursor": 3 })),
            ),
            (
                "dispatch_target_continue",
                Ok(json!({ "accepted": true, "state": "queued" })),
            ),
            ("dispatch_target_append", Ok(json!({ "accepted": true }))),
            ("dispatch_target_cancel", Ok(json!({ "cancelled": true }))),
        ]);

        let status = status_shared_assistant(
            &owner,
            &store,
            DispatchStatusRequest {
                job_id: JOB.to_string(),
                cursor: 3,
            },
        )
        .await
        .unwrap();
        assert_eq!(status["state"], "running");
        assert_eq!(store.get(JOB).await.unwrap().unwrap().last_cursor, 3);

        let follow_up: DispatchContinueRequest = serde_json::from_value(json!({
            "jobId": JOB,
            "turnId": "turn-2",
            "prompt": "And the open questions?",
            "model": "my-model",
            "reasoningPreset": "high",
            "approvalPolicy": "auto",
        }))
        .unwrap();
        continue_shared_assistant(&owner, &store, follow_up)
            .await
            .unwrap();
        let sent = owner.received("dispatch_target_continue");
        assert_eq!(sent["prompt"], "And the open questions?");
        for field in ["model", "reasoningPreset", "approvalPolicy"] {
            assert!(sent.get(field).is_none(), "{field} is the owner's");
        }

        let message: DispatchAppendRequest = serde_json::from_value(json!({
            "jobId": JOB,
            "messageId": "message-1",
            "content": "Keep it short",
        }))
        .unwrap();
        append_shared_assistant(&owner, &store, message)
            .await
            .unwrap();
        assert_eq!(
            owner.received("dispatch_target_append")["content"],
            "Keep it short"
        );

        cancel_shared_assistant(
            &owner,
            &store,
            DispatchJobRequest {
                job_id: JOB.to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            store.get(JOB).await.unwrap().unwrap().last_state,
            "cancelled"
        );
    }

    #[tokio::test]
    async fn a_conversation_the_owner_no_longer_keeps_is_expired_and_withdrawn_access_is_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let store = submitted(dir.path()).await;
        let owner = Owner::new(vec![
            (
                "dispatch_target_status",
                Err(anyhow!("dispatch job '{JOB}' does not exist")),
            ),
            (
                "dispatch_target_continue",
                Err(anyhow!(
                    "this device has no access to assistant 'workspace-notes'; request it again"
                )),
            ),
            (
                "dispatch_target_append",
                Err(SharedAssistantProblem::Revoked.into()),
            ),
        ]);

        let error = status_shared_assistant(
            &owner,
            &store,
            DispatchStatusRequest {
                job_id: JOB.to_string(),
                cursor: 0,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(problem(&error), Some(&SharedAssistantProblem::Expired));
        assert_eq!(store.get(JOB).await.unwrap().unwrap().last_state, "expired");

        let follow_up: DispatchContinueRequest = serde_json::from_value(json!({
            "jobId": JOB, "turnId": "turn-2", "prompt": "Again",
        }))
        .unwrap();
        let error = continue_shared_assistant(&owner, &store, follow_up)
            .await
            .unwrap_err();
        assert_eq!(problem(&error), Some(&SharedAssistantProblem::Revoked));

        let message: DispatchAppendRequest = serde_json::from_value(json!({
            "jobId": JOB, "messageId": "message-1", "content": "Hello",
        }))
        .unwrap();
        let error = append_shared_assistant(&owner, &store, message)
            .await
            .unwrap_err();
        assert!(error.to_string().starts_with("[shared-assistant:revoked]"));
    }

    #[tokio::test]
    async fn the_conversations_of_one_assistant_are_listed_and_kept_until_it_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutboundDispatchStore::new_in_root_for_tests(dir.path().to_path_buf());
        let entry = |job: &str, assistant: &str, state: &str| {
            json!({
                "jobId": job,
                "sessionId": SESSION,
                "state": state,
                "startedAt": "2026-11-01T08:00:00Z",
                "workspacePath": "",
                "title": format!("About {job}"),
                "agentType": format!("assistant:{assistant}"),
                "approvalPolicy": "remote",
                "delegation": { "assistantId": assistant },
            })
        };
        let owner = Owner::new(vec![(
            "dispatch_target_list",
            Ok(json!([
                entry("delegation-a", "workspace-notes", "succeeded"),
                entry("delegation-b", "workspace-other", "running"),
            ])),
        )]);
        let listed = list_shared_assistant_jobs(
            &owner,
            &store,
            "Notes".to_string(),
            DispatchListJobsRequest {
                target: Some(target()),
            },
        )
        .await
        .unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["jobId"], "delegation-a");
        let records = store.list().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title.as_deref(), Some("About delegation-a"));
        assert_eq!(records[0].last_state, "succeeded");
        assert!(records[0].target.is_shared_assistant());

        // A conversation another target already holds is not taken over.
        let clash = Owner::new(vec![(
            "dispatch_target_list",
            Ok(json!([entry(
                "delegation-a",
                "workspace-notes-2",
                "succeeded"
            )])),
        )]);
        let other = DispatchTargetRequest::SharedAssistant {
            owner_device_id: OWNER.to_string(),
            assistant_id: "workspace-notes-2".to_string(),
        };
        assert!(list_shared_assistant_jobs(
            &clash,
            &store,
            "Notes".to_string(),
            DispatchListJobsRequest {
                target: Some(other)
            },
        )
        .await
        .is_err());

        assert_eq!(
            forget_shared_assistant(&store, OWNER, "workspace-other")
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            forget_shared_assistant(&store, OWNER, "workspace-notes")
                .await
                .unwrap(),
            1
        );
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn approvals_and_result_sync_are_not_offered_for_shared_assistants() {
        let dir = tempfile::tempdir().unwrap();
        let store = submitted(dir.path()).await;
        let owner = Owner::new(Vec::new());
        let answer: super::super::DispatchAnswerRequest = serde_json::from_value(json!({
            "jobId": JOB, "requestId": "r1", "reply": "once",
        }))
        .unwrap();
        assert!(super::super::answer_device_dispatch(&owner, &store, answer)
            .await
            .is_err());
        assert!(owner.received.lock().unwrap().is_empty());
    }
}
