//! Assistants other devices share with this one, as subagents this device's
//! agents can call (F002-13).
//!
//! [`SharedAssistantDirectory`] implements the kernel's
//! [`RemoteAgentDelegationPort`] over the LAN share channel: it lists the
//! assistants shared with this device with whether each can take work, and
//! runs a call as a delegated job on the owner's device, the same job a
//! conversation with the assistant is (see
//! [`crate::service::dispatch::submit_shared_assistant`]). Unlike a
//! conversation, a call writes no outbound record here: only the calling
//! subagent observes it, and it is left out of this device's conversations
//! with the assistant.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use openbitfun_runtime_ports::{
    PermissionReply, PortError, PortErrorKind, PortResult, RemoteAgentAddress,
    RemoteAgentAvailability, RemoteAgentDelegationPort, RemoteAgentDescriptor, RemoteAgentJob,
    RemoteAgentJobSnapshot, RemoteAgentJobState, RemoteAgentListing, RemoteAgentSubmitRequest,
    RemoteAgentUnavailableReason,
};
use openbitfun_services_core::dispatch_contract::DISPATCH_PROTOCOL_VERSION;
use serde_json::{json, Value};

use super::host::{LanShareHost, SharedWithMeNow};
use super::shared_with_me::SharedWithMeState;
use crate::service::delegation::{assistant_agent_type, DELEGATED_JOB_ID_PREFIX};
use crate::service::dispatch::{
    admit_shared_assistant, explain_shared_assistant_error, validate_submit_ack, DeviceDispatchRpc,
    SharedAssistantProblem, SHARED_ASSISTANT_APPROVAL_POLICY,
};

/// How long a refusal from the owner's device stays in the listing after a
/// call, unless a later call goes through.
const REFUSAL_TTL: Duration = Duration::from_secs(5 * 60);
/// The owner's runner says this when it has no executor for tasks.
const EXECUTOR_MISSING_TEXT: &str = "its task runner is missing";

/// The assistants shared with this device and whether each owner can be
/// reached now.
#[async_trait]
pub trait SharedWithMePresence: Send + Sync {
    async fn shared_with_me_now(&self) -> Result<Vec<SharedWithMeNow>, String>;

    /// This device's id, as owners know it.
    fn local_device_id(&self) -> Option<String> {
        None
    }
}

#[async_trait]
impl SharedWithMePresence for LanShareHost {
    async fn shared_with_me_now(&self) -> Result<Vec<SharedWithMeNow>, String> {
        LanShareHost::shared_with_me_now(self).await
    }

    fn local_device_id(&self) -> Option<String> {
        Some(self.identity().device_id().to_string())
    }
}

/// Why the owner's device refused the last call, until it expires.
struct Refusal {
    at: Instant,
    reason: RemoteAgentUnavailableReason,
    detail: String,
}

/// The assistants shared with this device, as callable subagents.
pub struct SharedAssistantDirectory {
    presence: Arc<dyn SharedWithMePresence>,
    rpc: Arc<dyn DeviceDispatchRpc>,
    refusals: Mutex<HashMap<RemoteAgentAddress, Refusal>>,
}

impl SharedAssistantDirectory {
    pub fn new(presence: Arc<dyn SharedWithMePresence>, rpc: Arc<dyn DeviceDispatchRpc>) -> Self {
        Self {
            presence,
            rpc,
            refusals: Mutex::new(HashMap::new()),
        }
    }

    fn remember_refusal(&self, address: &RemoteAgentAddress, problem: &anyhow::Error) {
        let Some((reason, detail)) = unavailable_reason(problem) else {
            return;
        };
        self.refusals.lock().expect("refusals lock").insert(
            address.clone(),
            Refusal {
                at: Instant::now(),
                reason,
                detail,
            },
        );
    }

    fn forget_refusal(&self, address: &RemoteAgentAddress) {
        self.refusals.lock().expect("refusals lock").remove(address);
    }

    fn recent_refusal(&self, address: &RemoteAgentAddress) -> Option<RemoteAgentAvailability> {
        let mut refusals = self.refusals.lock().expect("refusals lock");
        refusals.retain(|_, refusal| refusal.at.elapsed() < REFUSAL_TTL);
        refusals
            .get(address)
            .map(|refusal| RemoteAgentAvailability::Unavailable {
                reason: refusal.reason,
                detail: refusal.detail.clone(),
            })
    }

    fn descriptor(&self, entry: SharedWithMeNow) -> RemoteAgentDescriptor {
        let SharedWithMeNow { assistant, online } = entry;
        let address = RemoteAgentAddress {
            device_id: assistant.owner.device_id.clone(),
            assistant_id: assistant.assistant_id.clone(),
        };
        let owner = assistant.owner.display_name.clone();
        let availability = if assistant.state == SharedWithMeState::Revoked {
            RemoteAgentAvailability::Unavailable {
                reason: RemoteAgentUnavailableReason::Revoked,
                detail: format!("{owner} no longer shares this assistant with this device"),
            }
        } else if !online {
            RemoteAgentAvailability::Unavailable {
                reason: RemoteAgentUnavailableReason::Offline,
                detail: format!("{owner}'s device is not reachable on the local network"),
            }
        } else {
            self.recent_refusal(&address)
                .unwrap_or(RemoteAgentAvailability::Available)
        };
        RemoteAgentDescriptor {
            agent_type: assistant_agent_type(&assistant.assistant_id),
            address,
            device_name: owner,
            display_name: assistant.name,
            description: assistant.description,
            availability,
        }
    }
}

/// The listed reason for a refusal that will hold for the next call too.
fn unavailable_reason(error: &anyhow::Error) -> Option<(RemoteAgentUnavailableReason, String)> {
    let reason = match error.downcast_ref::<SharedAssistantProblem>() {
        Some(SharedAssistantProblem::NotShared | SharedAssistantProblem::Revoked) => {
            RemoteAgentUnavailableReason::Revoked
        }
        Some(SharedAssistantProblem::NeedsUpdate(_)) => RemoteAgentUnavailableReason::Incompatible,
        Some(SharedAssistantProblem::Unavailable(_)) => {
            RemoteAgentUnavailableReason::AgentUnavailable
        }
        // Presence already lists an unreachable owner; a running conversation
        // says nothing about the assistant.
        Some(
            SharedAssistantProblem::Offline(_)
            | SharedAssistantProblem::Expired
            | SharedAssistantProblem::Active,
        ) => return None,
        None if format!("{error:#}").contains(EXECUTOR_MISSING_TEXT) => {
            RemoteAgentUnavailableReason::ExecutorMissing
        }
        None => return None,
    };
    Some((reason, format!("{error:#}")))
}

/// The owner's refusal reads as the reason, so it is passed through.
fn port_error(error: anyhow::Error) -> PortError {
    let kind = match error.downcast_ref::<SharedAssistantProblem>() {
        Some(SharedAssistantProblem::Offline(_)) => PortErrorKind::NotAvailable,
        Some(SharedAssistantProblem::NotShared | SharedAssistantProblem::Revoked) => {
            PortErrorKind::PermissionDenied
        }
        Some(SharedAssistantProblem::Expired) => PortErrorKind::NotFound,
        _ => PortErrorKind::Backend,
    };
    PortError::new(kind, format!("{error:#}"))
}

/// A job the owner will never run again: it withdrew access, or no longer
/// keeps the job. Observing it again would wait forever.
fn ended_for_good(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<SharedAssistantProblem>(),
        Some(
            SharedAssistantProblem::NotShared
                | SharedAssistantProblem::Revoked
                | SharedAssistantProblem::Expired
        )
    )
}

/// Names the shared assistant and the calling session for the owner's
/// admission and its list of tasks from other devices.
fn delegation_block(request: &RemoteAgentSubmitRequest) -> Value {
    let mut source = json!({
        "sessionId": request.source.session_id,
        "agentType": request.source.agent_type,
    });
    if let Some(display_name) = &request.source.display_name {
        source["displayName"] = Value::String(display_name.clone());
    }
    json!({
        "assistantId": request.address.assistant_id,
        "source": source,
        "callChain": request.call_chain,
    })
}

#[async_trait]
impl RemoteAgentDelegationPort for SharedAssistantDirectory {
    async fn list_agents(&self, _listing: RemoteAgentListing) -> Vec<RemoteAgentDescriptor> {
        // Presence is kept current by the share host, so every listing is
        // fresh without asking each owner.
        match self.presence.shared_with_me_now().await {
            Ok(entries) => entries
                .into_iter()
                .map(|entry| self.descriptor(entry))
                .collect(),
            Err(error) => {
                log::warn!("Failed to list assistants shared with this device: {error}");
                Vec::new()
            }
        }
    }

    fn local_device_id(&self) -> Option<String> {
        self.presence.local_device_id()
    }

    async fn submit(&self, request: RemoteAgentSubmitRequest) -> PortResult<RemoteAgentJob> {
        if !request.job_id.starts_with(DELEGATED_JOB_ID_PREFIX) {
            return Err(PortError::new(
                PortErrorKind::InvalidRequest,
                format!("calls to shared assistants use '{DELEGATED_JOB_ID_PREFIX}' job ids"),
            ));
        }
        let address = &request.address;
        let agent = match admit_shared_assistant(
            self.rpc.as_ref(),
            &address.device_id,
            &address.assistant_id,
        )
        .await
        {
            Ok(agent) => agent,
            Err(error) => {
                self.remember_refusal(address, &error);
                return Err(port_error(error));
            }
        };
        let mut payload = json!({
            "protocolVersion": DISPATCH_PROTOCOL_VERSION,
            "jobId": request.job_id,
            "sessionId": request.session_id,
            "workspacePath": "",
            "agentType": agent.agent_type,
            "prompt": request.prompt,
            "approvalPolicy": SHARED_ASSISTANT_APPROVAL_POLICY,
            "delegation": delegation_block(&request),
        });
        if let Some(title) = request
            .title
            .as_ref()
            .filter(|title| !title.trim().is_empty())
        {
            payload["title"] = Value::String(title.clone());
        }
        let response = match self
            .rpc
            .invoke(&address.device_id, "dispatch_target_submit", payload)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let error = explain_shared_assistant_error(error, false);
                self.remember_refusal(address, &error);
                return Err(port_error(error));
            }
        };
        validate_submit_ack(&response, &request.job_id, &request.session_id).map_err(port_error)?;
        self.forget_refusal(address);
        Ok(RemoteAgentJob {
            device_id: request.address.device_id,
            job_id: request.job_id,
            session_id: request.session_id,
        })
    }

    async fn observe(
        &self,
        job: &RemoteAgentJob,
        cursor: u64,
    ) -> PortResult<RemoteAgentJobSnapshot> {
        let response = match self
            .rpc
            .invoke(
                &job.device_id,
                "dispatch_target_status",
                json!({ "jobId": job.job_id, "cursor": cursor }),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let error = explain_shared_assistant_error(error, true);
                if ended_for_good(&error) {
                    return Ok(RemoteAgentJobSnapshot {
                        state: RemoteAgentJobState::Failed,
                        cursor,
                        events: Vec::new(),
                        pending_permissions: Vec::new(),
                        cursor_reset: false,
                        last_error: Some(format!("{error:#}")),
                        hold: None,
                    });
                }
                return Err(port_error(error));
            }
        };
        serde_json::from_value(response).map_err(|error| {
            PortError::new(
                PortErrorKind::Backend,
                format!("the owner's device returned an unreadable task status: {error}"),
            )
        })
    }

    async fn cancel(&self, job: &RemoteAgentJob) -> PortResult<bool> {
        let response = self
            .rpc
            .invoke(
                &job.device_id,
                "dispatch_target_cancel",
                json!({ "jobId": job.job_id }),
            )
            .await
            .map_err(|error| port_error(explain_shared_assistant_error(error, true)))?;
        Ok(response
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    async fn answer(
        &self,
        _job: &RemoteAgentJob,
        _request_id: &str,
        _reply: PermissionReply,
    ) -> PortResult<()> {
        // The owner of the device the assistant runs on answers its approvals.
        Err(PortError::new(
            PortErrorKind::PermissionDenied,
            "the owner of the shared assistant answers its approvals on their device",
        ))
    }

    async fn continue_job(
        &self,
        job: &RemoteAgentJob,
        turn_id: &str,
        prompt: String,
    ) -> PortResult<()> {
        self.rpc
            .invoke(
                &job.device_id,
                "dispatch_target_continue",
                json!({
                    "protocolVersion": DISPATCH_PROTOCOL_VERSION,
                    "jobId": job.job_id,
                    "turnId": turn_id,
                    "prompt": prompt,
                }),
            )
            .await
            .map(|_| ())
            .map_err(|error| port_error(explain_shared_assistant_error(error, true)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::lan_share::{SharePeer, SharedWithMe};
    use openbitfun_services_core::dispatch_contract::DISPATCH_AGENT_DELEGATION_CAPABILITY;

    struct Presence(Vec<SharedWithMeNow>);

    #[async_trait]
    impl SharedWithMePresence for Presence {
        async fn shared_with_me_now(&self) -> Result<Vec<SharedWithMeNow>, String> {
            Ok(self.0.clone())
        }
    }

    /// An owner's device that answers each command from a script and
    /// records what it was sent.
    #[derive(Default)]
    struct Owner {
        answers: Mutex<HashMap<String, Result<Value, String>>>,
        problem: Mutex<Option<fn() -> SharedAssistantProblem>>,
        sent: Mutex<Vec<(String, Value)>>,
    }

    impl Owner {
        fn answer(&self, command: &str, answer: Result<Value, String>) {
            self.answers
                .lock()
                .unwrap()
                .insert(command.to_string(), answer);
        }

        fn sent(&self, command: &str) -> Vec<Value> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .filter(|(sent, _)| sent == command)
                .map(|(_, args)| args.clone())
                .collect()
        }
    }

    #[async_trait]
    impl DeviceDispatchRpc for Owner {
        async fn invoke(
            &self,
            _device_id: &str,
            command: &str,
            args: Value,
        ) -> anyhow::Result<Value> {
            self.sent.lock().unwrap().push((command.to_string(), args));
            if let Some(problem) = *self.problem.lock().unwrap() {
                return Err(problem().into());
            }
            match self.answers.lock().unwrap().get(command).cloned() {
                Some(Ok(value)) => Ok(value),
                Some(Err(message)) => Err(anyhow::anyhow!(message)),
                None => Err(anyhow::anyhow!("unexpected command {command}")),
            }
        }
    }

    fn owner_that_takes_tasks() -> Arc<Owner> {
        let owner = Arc::new(Owner::default());
        let mut capabilities: Vec<_> =
            openbitfun_services_core::dispatch_contract::dispatch_required_target_capabilities()
                .collect();
        capabilities.push(DISPATCH_AGENT_DELEGATION_CAPABILITY);
        owner.answer(
            "dispatch_target_probe",
            Ok(json!({
                "productId": openbitfun_services_core::product_identity::product_id(),
                "dataNamespace": openbitfun_services_core::product_identity::data_namespace(),
                "protocolVersion": DISPATCH_PROTOCOL_VERSION,
                "capabilities": capabilities,
            })),
        );
        owner.answer(
            "dispatch_target_catalog",
            Ok(json!({ "agents": [{
                "id": "workspace",
                "agentType": "assistant:workspace",
                "displayName": "Writer",
            }] })),
        );
        owner.answer(
            "dispatch_target_submit",
            Ok(json!({ "accepted": true, "jobId": "delegation-1", "sessionId": "session-1", "state": "queued" })),
        );
        owner
    }

    fn shared(assistant_id: &str, online: bool, state: SharedWithMeState) -> SharedWithMeNow {
        SharedWithMeNow {
            assistant: SharedWithMe {
                owner: SharePeer {
                    public_key: "a".repeat(64),
                    device_id: "ann-device".to_string(),
                    fingerprint: "ABCD".to_string(),
                    display_name: "Ann".to_string(),
                    device_name: "Ann's Mac".to_string(),
                },
                assistant_id: assistant_id.to_string(),
                name: "Writer".to_string(),
                description: Some("Writes with care".to_string()),
                granted_at: "2026-11-01T00:00:00Z".to_string(),
                state,
                last_address: None,
            },
            online,
        }
    }

    fn directory(entries: Vec<SharedWithMeNow>, owner: Arc<Owner>) -> SharedAssistantDirectory {
        SharedAssistantDirectory::new(Arc::new(Presence(entries)), owner)
    }

    fn request(assistant_id: &str) -> RemoteAgentSubmitRequest {
        RemoteAgentSubmitRequest {
            address: RemoteAgentAddress {
                device_id: "ann-device".to_string(),
                assistant_id: assistant_id.to_string(),
            },
            job_id: "delegation-1".to_string(),
            session_id: "session-1".to_string(),
            prompt: "Draft the notes".to_string(),
            title: Some("Notes".to_string()),
            source: openbitfun_runtime_ports::RemoteAgentSource {
                session_id: "caller-session".to_string(),
                agent_type: "assistant:workspace-x".to_string(),
                display_name: Some("X".to_string()),
            },
            call_chain: vec!["me-device/workspace-x".to_string()],
        }
    }

    fn reasons(listed: &[RemoteAgentDescriptor]) -> Vec<Option<RemoteAgentUnavailableReason>> {
        listed
            .iter()
            .map(|descriptor| match &descriptor.availability {
                RemoteAgentAvailability::Available => None,
                RemoteAgentAvailability::Unavailable { reason, .. } => Some(*reason),
            })
            .collect()
    }

    #[tokio::test]
    async fn assistants_are_listed_with_their_owner_and_why_they_cannot_take_work() {
        let listed = directory(
            vec![
                shared("workspace", true, SharedWithMeState::Available),
                shared("workspace-away", false, SharedWithMeState::Available),
                shared("workspace-gone", true, SharedWithMeState::Revoked),
            ],
            Arc::new(Owner::default()),
        )
        .list_agents(RemoteAgentListing::Cached)
        .await;
        assert_eq!(
            listed[0].address.agent_type(),
            "shared:ann-device/workspace"
        );
        assert_eq!(listed[0].device_name, "Ann");
        assert_eq!(listed[0].agent_type, "assistant:workspace");
        assert_eq!(listed[0].description.as_deref(), Some("Writes with care"));
        assert_eq!(
            reasons(&listed),
            [
                None,
                Some(RemoteAgentUnavailableReason::Offline),
                Some(RemoteAgentUnavailableReason::Revoked),
            ]
        );
    }

    #[tokio::test]
    async fn a_call_is_a_delegated_job_naming_the_calling_session() {
        let owner = owner_that_takes_tasks();
        let directory = directory(
            vec![shared("workspace", true, SharedWithMeState::Available)],
            owner.clone(),
        );
        let job = directory
            .submit(request("workspace"))
            .await
            .expect("submitted");
        assert_eq!(job.device_id, "ann-device");
        let submitted = owner.sent("dispatch_target_submit");
        assert_eq!(submitted.len(), 1);
        let payload = &submitted[0];
        assert_eq!(payload["agentType"], "assistant:workspace");
        assert_eq!(payload["approvalPolicy"], "remote");
        assert_eq!(payload["workspacePath"], "");
        assert_eq!(payload["delegation"]["assistantId"], "workspace");
        assert_eq!(
            payload["delegation"]["source"]["agentType"],
            "assistant:workspace-x"
        );
        assert_eq!(
            payload["delegation"]["source"]["sessionId"],
            "caller-session"
        );
        assert_eq!(
            payload["delegation"]["callChain"],
            json!(["me-device/workspace-x"])
        );

        let mut wrong = request("workspace");
        wrong.job_id = "job-1".to_string();
        let error = directory.submit(wrong).await.unwrap_err();
        assert_eq!(error.kind, PortErrorKind::InvalidRequest);
    }

    #[tokio::test]
    async fn a_refused_call_is_listed_with_its_reason_until_one_goes_through() {
        let owner = owner_that_takes_tasks();
        owner.answer(
            "dispatch_target_probe",
            Err("this device cannot run tasks for its shared assistants: its task runner is missing"
                .to_string()),
        );
        let directory = directory(
            vec![shared("workspace", true, SharedWithMeState::Available)],
            owner.clone(),
        );
        let error = directory.submit(request("workspace")).await.unwrap_err();
        assert!(
            error.message.contains("task runner is missing"),
            "{}",
            error.message
        );
        assert!(owner.sent("dispatch_target_submit").is_empty());
        let listed = directory.list_agents(RemoteAgentListing::Cached).await;
        assert_eq!(
            reasons(&listed),
            [Some(RemoteAgentUnavailableReason::ExecutorMissing)]
        );

        let owner_back = owner_that_takes_tasks();
        let directory = SharedAssistantDirectory {
            rpc: owner_back,
            ..directory
        };
        directory
            .submit(request("workspace"))
            .await
            .expect("submitted");
        let listed = directory.list_agents(RemoteAgentListing::Cached).await;
        assert_eq!(reasons(&listed), [None]);

        // An owner that withdrew access is listed as revoked after the call.
        let withdrawn = owner_that_takes_tasks();
        *withdrawn.problem.lock().unwrap() = Some(|| SharedAssistantProblem::Revoked);
        let directory = SharedAssistantDirectory {
            rpc: withdrawn,
            ..directory
        };
        let error = directory.submit(request("workspace")).await.unwrap_err();
        assert_eq!(error.kind, PortErrorKind::PermissionDenied);
        let listed = directory.list_agents(RemoteAgentListing::Cached).await;
        assert_eq!(
            reasons(&listed),
            [Some(RemoteAgentUnavailableReason::Revoked)]
        );
    }

    #[tokio::test]
    async fn a_job_the_owner_will_never_run_again_ends_instead_of_being_observed_forever() {
        let job = RemoteAgentJob {
            device_id: "ann-device".to_string(),
            job_id: "delegation-1".to_string(),
            session_id: "session-1".to_string(),
        };
        let owner = Arc::new(Owner::default());
        *owner.problem.lock().unwrap() = Some(|| SharedAssistantProblem::Revoked);
        let directory = directory(Vec::new(), owner);
        let snapshot = directory.observe(&job, 7).await.expect("a final snapshot");
        assert_eq!(snapshot.state, RemoteAgentJobState::Failed);
        assert_eq!(snapshot.cursor, 7);
        assert!(snapshot
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("[shared-assistant:revoked]")));

        // An owner that is only away is observed again later.
        let away = Arc::new(Owner::default());
        *away.problem.lock().unwrap() =
            Some(|| SharedAssistantProblem::Offline("timed out".into()));
        let directory = SharedAssistantDirectory {
            rpc: away,
            ..directory
        };
        let error = directory.observe(&job, 7).await.unwrap_err();
        assert_eq!(error.kind, PortErrorKind::NotAvailable);

        // Only the owner answers the approvals of its assistant.
        let error = directory
            .answer(&job, "request-1", PermissionReply::Once)
            .await
            .unwrap_err();
        assert_eq!(error.kind, PortErrorKind::PermissionDenied);
    }
}
