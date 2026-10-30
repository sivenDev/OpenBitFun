//! Requester side of agent delegation: the agents other devices of this
//! account opened to this device, reached through account-device dispatch.
//!
//! Hosts supply the device RPC and the account's device list; this module
//! decides which devices can take delegated work and what they offer. A device
//! is queried only when it is online and advertises agent delegation plus the
//! dispatch surface this build requires, so an older device is reported as
//! incompatible instead of failing later.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use openbitfun_runtime_ports::{
    PortError, PortErrorKind, PortResult, RemoteAgentAddress, RemoteAgentAvailability,
    RemoteAgentCodingResult, RemoteAgentDescriptor, RemoteAgentListing, RemoteAgentTaskKind,
    RemoteAgentUnavailableReason,
};
use openbitfun_services_core::dispatch_contract::{
    DispatchCatalogAgent, DispatchCatalogResponse, DISPATCH_AGENT_DELEGATION_CAPABILITY,
    DISPATCH_EXTERNAL_AGENT_EXECUTOR_CAPABILITY,
};
use openbitfun_services_integrations::remote_ssh::dispatch_ssh;
use serde_json::{json, Value};

use super::device_controller::DeviceDispatchRpc;
use super::OutboundDispatchStore;

/// How long a listing may reuse a device's last answer.
const CATALOG_CACHE_TTL: Duration = Duration::from_secs(30);
/// Bound on probing and listing one device, so a slow device cannot stall
/// every turn's catalog.
const DEVICE_QUERY_TIMEOUT: Duration = Duration::from_secs(8);
/// Delegated jobs always ask the target owner for approval.
const DELEGATED_APPROVAL_POLICY: &str = "remote";
/// Commits work a coding job left uncommitted before its result is synced.
const DELEGATED_RESULT_COMMIT_MESSAGE: &str = "OpenBitFun delegated task: uncommitted changes";

/// One other device of the signed-in account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDevice {
    pub device_id: String,
    pub device_name: String,
    pub online: bool,
}

/// The signed-in account's devices, as the host's account client sees them.
#[async_trait]
pub trait AccountDeviceDirectory: Send + Sync {
    /// Devices of this account other than this one. Fails when no account is
    /// signed in or the relay cannot be reached.
    async fn other_devices(&self) -> anyhow::Result<Vec<AccountDevice>>;
}

#[derive(Clone)]
struct DeviceCatalog {
    fetched_at: Instant,
    device_name: String,
    /// The device's last successful answer, kept so its agents stay listed
    /// with a reason while it is offline or unreachable.
    agents: Vec<DispatchCatalogAgent>,
    problem: Option<(RemoteAgentUnavailableReason, String)>,
}

/// Lists and caches the agents other devices opened to this device.
pub struct RemoteAgentDirectory {
    rpc: Arc<dyn DeviceDispatchRpc>,
    devices: Arc<dyn AccountDeviceDirectory>,
    cache: tokio::sync::Mutex<HashMap<String, DeviceCatalog>>,
    /// Owns the baseline of delegated coding jobs; without it this host
    /// delegates message tasks only.
    outbound: Option<OutboundDispatchStore>,
}

impl RemoteAgentDirectory {
    pub fn new(rpc: Arc<dyn DeviceDispatchRpc>, devices: Arc<dyn AccountDeviceDirectory>) -> Self {
        Self {
            rpc,
            devices,
            cache: tokio::sync::Mutex::new(HashMap::new()),
            outbound: None,
        }
    }

    /// Enables coding tasks. Their Git baseline and result sync reuse device
    /// dispatch, which records each job in `store`.
    pub fn with_outbound_store(mut self, store: OutboundDispatchStore) -> Self {
        self.outbound = Some(store);
        self
    }

    pub async fn list(&self, listing: RemoteAgentListing) -> Vec<RemoteAgentDescriptor> {
        let devices = match self.devices.other_devices().await {
            Ok(devices) => devices,
            Err(error) => {
                log::debug!("Remote agents not listed: account devices unavailable: {error}");
                return Vec::new();
            }
        };
        let previous = self.cache.lock().await.clone();
        let catalogs = futures::future::join_all(devices.into_iter().map(|device| {
            let previous = previous.get(&device.device_id).cloned();
            async move {
                let catalog = match previous {
                    Some(cached)
                        if listing == RemoteAgentListing::Cached
                            && device.online
                            && cached.problem.is_none()
                            && cached.fetched_at.elapsed() < CATALOG_CACHE_TTL =>
                    {
                        cached
                    }
                    previous => self.query_device(&device, previous).await,
                };
                (device.device_id, catalog)
            }
        }))
        .await;

        let mut cache = self.cache.lock().await;
        let mut descriptors = Vec::new();
        for (device_id, catalog) in catalogs {
            descriptors.extend(descriptors_for(&device_id, &catalog));
            cache.insert(device_id, catalog);
        }
        descriptors
    }

    /// The descriptor for `address` from the freshest listing.
    pub async fn find(&self, address: &RemoteAgentAddress) -> Option<RemoteAgentDescriptor> {
        self.list(RemoteAgentListing::Cached)
            .await
            .into_iter()
            .find(|descriptor| &descriptor.address == address)
    }

    async fn query_device(
        &self,
        device: &AccountDevice,
        previous: Option<DeviceCatalog>,
    ) -> DeviceCatalog {
        let known_agents = previous.map(|catalog| catalog.agents).unwrap_or_default();
        let unavailable = |reason, detail: String| DeviceCatalog {
            fetched_at: Instant::now(),
            device_name: device.device_name.clone(),
            agents: known_agents.clone(),
            problem: Some((reason, detail)),
        };
        if !device.online {
            return unavailable(
                RemoteAgentUnavailableReason::Offline,
                format!("{} is offline", device.device_name),
            );
        }
        let answer = tokio::time::timeout(DEVICE_QUERY_TIMEOUT, self.fetch_catalog(device)).await;
        match answer {
            Ok(Ok(agents)) => DeviceCatalog {
                fetched_at: Instant::now(),
                device_name: device.device_name.clone(),
                agents,
                problem: None,
            },
            Ok(Err(CatalogError::Incompatible(detail))) => {
                unavailable(RemoteAgentUnavailableReason::Incompatible, detail)
            }
            Ok(Err(CatalogError::Unreachable(detail))) => {
                unavailable(RemoteAgentUnavailableReason::Unreachable, detail)
            }
            Err(_) => unavailable(
                RemoteAgentUnavailableReason::Unreachable,
                format!("{} did not answer in time", device.device_name),
            ),
        }
    }

    async fn fetch_catalog(
        &self,
        device: &AccountDevice,
    ) -> Result<Vec<DispatchCatalogAgent>, CatalogError> {
        let protocol = self
            .rpc
            .invoke(&device.device_id, "dispatch_target_probe", json!({}))
            .await
            .map_err(|error| CatalogError::Unreachable(format!("{error:#}")))?;
        ensure_delegation_target(&protocol, &device.device_name)?;
        let runs_external_agents =
            has_capability(&protocol, DISPATCH_EXTERNAL_AGENT_EXECUTOR_CAPABILITY);
        let catalog = self
            .rpc
            .invoke(&device.device_id, "dispatch_target_catalog", json!({}))
            .await
            .map_err(|error| CatalogError::Unreachable(format!("{error:#}")))?;
        serde_json::from_value::<DispatchCatalogResponse>(catalog)
            // A target that cannot run external agents never lists them.
            .map(|catalog| {
                catalog
                    .agents
                    .into_iter()
                    .filter(|agent| {
                        runs_external_agents
                            || !crate::agentic::coordination::is_external_agent_type(
                                &agent.agent_type,
                            )
                    })
                    .collect()
            })
            .map_err(|error| {
                CatalogError::Incompatible(format!(
                    "{} returned an unreadable agent catalog: {error}",
                    device.device_name
                ))
            })
    }
}

#[async_trait]
impl openbitfun_runtime_ports::RemoteAgentDelegationPort for RemoteAgentDirectory {
    async fn list_agents(&self, listing: RemoteAgentListing) -> Vec<RemoteAgentDescriptor> {
        self.list(listing).await
    }

    async fn submit(
        &self,
        request: openbitfun_runtime_ports::RemoteAgentSubmitRequest,
    ) -> PortResult<openbitfun_runtime_ports::RemoteAgentJob> {
        let agent_type = request.address.agent_type();
        let descriptor = self.find(&request.address).await.ok_or_else(|| {
            PortError::new(
                PortErrorKind::NotFound,
                format!("{agent_type} is not open to this device"),
            )
        })?;
        if let RemoteAgentAvailability::Unavailable { detail, .. } = &descriptor.availability {
            return Err(PortError::new(PortErrorKind::NotAvailable, detail.clone()));
        }
        if !descriptor.task_kinds.contains(&request.task_kind) {
            return Err(PortError::new(
                PortErrorKind::InvalidRequest,
                format!("{agent_type} does not accept {:?} tasks", request.task_kind),
            ));
        }
        if request.task_kind == RemoteAgentTaskKind::Coding {
            return self.submit_coding(request, &descriptor).await;
        }
        let payload = delegated_submit_payload(&request, &descriptor);
        let response = self
            .rpc
            .invoke(
                &request.address.device_id,
                "dispatch_target_submit",
                payload,
            )
            .await
            .map_err(target_error)?;
        super::controller::validate_submit_ack(&response, &request.job_id, &request.session_id)
            .map_err(target_error)?;
        Ok(openbitfun_runtime_ports::RemoteAgentJob {
            device_id: request.address.device_id,
            job_id: request.job_id,
            session_id: request.session_id,
        })
    }

    async fn observe(
        &self,
        job: &openbitfun_runtime_ports::RemoteAgentJob,
        cursor: u64,
    ) -> PortResult<openbitfun_runtime_ports::RemoteAgentJobSnapshot> {
        let response = self
            .rpc
            .invoke(
                &job.device_id,
                "dispatch_target_status",
                json!({ "jobId": job.job_id, "cursor": cursor }),
            )
            .await
            .map_err(target_error)?;
        serde_json::from_value(response).map_err(|error| {
            PortError::new(
                PortErrorKind::Backend,
                format!("the device returned an unreadable job status: {error}"),
            )
        })
    }

    async fn cancel(&self, job: &openbitfun_runtime_ports::RemoteAgentJob) -> PortResult<bool> {
        let response = self
            .rpc
            .invoke(
                &job.device_id,
                "dispatch_target_cancel",
                json!({ "jobId": job.job_id }),
            )
            .await
            .map_err(target_error)?;
        Ok(response
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    async fn answer(
        &self,
        job: &openbitfun_runtime_ports::RemoteAgentJob,
        request_id: &str,
        reply: openbitfun_runtime_ports::PermissionReply,
    ) -> PortResult<()> {
        // The target reads the reply flat beside the job and request ids.
        let mut payload = serde_json::to_value(&reply)
            .map_err(|error| PortError::new(PortErrorKind::InvalidRequest, error.to_string()))?;
        let Some(fields) = payload.as_object_mut() else {
            return Err(PortError::new(
                PortErrorKind::InvalidRequest,
                "permission reply is not an object",
            ));
        };
        fields.insert("jobId".into(), Value::String(job.job_id.clone()));
        fields.insert("requestId".into(), Value::String(request_id.to_string()));
        self.rpc
            .invoke(&job.device_id, "dispatch_target_answer", payload)
            .await
            .map(|_| ())
            .map_err(target_error)
    }

    async fn sync_coding_result(
        &self,
        job: &openbitfun_runtime_ports::RemoteAgentJob,
    ) -> PortResult<RemoteAgentCodingResult> {
        let store = self.outbound_store()?;
        let response = super::device_controller::sync_device_result(
            self.rpc.as_ref(),
            store,
            super::controller::DispatchSyncResultRequest {
                job_id: job.job_id.clone(),
                message: Some(DELEGATED_RESULT_COMMIT_MESSAGE.to_string()),
            },
        )
        .await
        .map_err(target_error)?;
        let worktree_path = match response.get("baselineWorktreePath").and_then(Value::as_str) {
            Some(path) => Some(path.to_string()),
            None => store
                .get(&job.job_id)
                .await
                .ok()
                .flatten()
                .and_then(|record| record.baseline_worktree_path),
        };
        coding_result(&response, worktree_path)
    }

    async fn continue_job(
        &self,
        job: &openbitfun_runtime_ports::RemoteAgentJob,
        turn_id: &str,
        prompt: String,
    ) -> PortResult<()> {
        self.rpc
            .invoke(
                &job.device_id,
                "dispatch_target_continue",
                json!({
                    "protocolVersion": super::controller::DISPATCH_PROTOCOL_VERSION,
                    "jobId": job.job_id,
                    "turnId": turn_id,
                    "prompt": prompt,
                }),
            )
            .await
            .map(|_| ())
            .map_err(target_error)
    }
}

impl RemoteAgentDirectory {
    fn outbound_store(&self) -> PortResult<&OutboundDispatchStore> {
        self.outbound.as_ref().ok_or_else(|| {
            PortError::new(
                PortErrorKind::NotAvailable,
                "this host cannot delegate coding tasks to other devices",
            )
        })
    }

    /// A coding job branches from a managed baseline worktree of the
    /// requesting workspace, exactly like a manual device dispatch, so its
    /// result comes back as a branch the user can inspect or discard.
    async fn submit_coding(
        &self,
        request: openbitfun_runtime_ports::RemoteAgentSubmitRequest,
        descriptor: &RemoteAgentDescriptor,
    ) -> PortResult<openbitfun_runtime_ports::RemoteAgentJob> {
        let store = self.outbound_store()?;
        let workspace = request
            .source_workspace_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| {
                PortError::new(
                    PortErrorKind::InvalidRequest,
                    "coding tasks need a session in a Git workspace on this device",
                )
            })?;
        crate::service::git::GitService::resolve_worktree_repository(std::path::Path::new(
            workspace,
        ))
        .await
        .map_err(|error| {
            PortError::new(
                PortErrorKind::InvalidRequest,
                format!("coding tasks need a session in a Git workspace on this device: {error}"),
            )
        })?;
        let submit = super::controller::DispatchSubmitRequest {
            target: super::DispatchTargetRequest::Device {
                device_id: request.address.device_id.clone(),
                workspace_path: String::new(),
            },
            base_ref: None,
            include_uncommitted: false,
            job_id: request.job_id.clone(),
            session_id: request.session_id.clone(),
            agent_type: descriptor.agent_type.clone(),
            prompt: request.prompt.clone(),
            approval_policy: DELEGATED_APPROVAL_POLICY.to_string(),
            model: None,
            reasoning_preset: None,
            title: request
                .title
                .clone()
                .filter(|title| !title.trim().is_empty()),
            source_workspace_path: Some(workspace.to_string()),
            source_workspace_id: None,
            attachments: Vec::new(),
        };
        super::device_controller::submit_device_job(
            self.rpc.as_ref(),
            store,
            descriptor.device_name.clone(),
            submit,
            Some(delegation_block(&request)),
        )
        .await
        .map_err(target_error)?;
        Ok(openbitfun_runtime_ports::RemoteAgentJob {
            device_id: request.address.device_id,
            job_id: request.job_id,
            session_id: request.session_id,
        })
    }
}

/// Names the open agent and the requesting session for the target's
/// admission and audit.
fn delegation_block(request: &openbitfun_runtime_ports::RemoteAgentSubmitRequest) -> Value {
    let task_kind = match request.task_kind {
        RemoteAgentTaskKind::Message => "message",
        RemoteAgentTaskKind::Coding => "coding",
    };
    let mut source = json!({
        "sessionId": request.source.session_id,
        "agentType": request.source.agent_type,
    });
    if let Some(display_name) = &request.source.display_name {
        source["displayName"] = Value::String(display_name.clone());
    }
    json!({
        "openAgentId": request.address.open_agent_id,
        "taskKind": task_kind,
        "source": source,
    })
}

/// Reads the target's sync answer into the port's result.
fn coding_result(
    response: &Value,
    worktree_path: Option<String>,
) -> PortResult<RemoteAgentCodingResult> {
    let mut result: RemoteAgentCodingResult =
        serde_json::from_value(response.clone()).map_err(|error| {
            PortError::new(
                PortErrorKind::Backend,
                format!("the device returned an unreadable sync result: {error}"),
            )
        })?;
    if let Some(head) = response.get("syncedHeadCommit").and_then(Value::as_str) {
        result.head_commit = head.to_string();
    }
    result.worktree_path = worktree_path;
    Ok(result)
}

/// A delegated message job. The target chooses where it runs, which agent
/// mode it uses, and asks its owner for every permission.
fn delegated_submit_payload(
    request: &openbitfun_runtime_ports::RemoteAgentSubmitRequest,
    descriptor: &RemoteAgentDescriptor,
) -> Value {
    let mut payload = json!({
        "protocolVersion": super::controller::DISPATCH_PROTOCOL_VERSION,
        "jobId": request.job_id,
        "sessionId": request.session_id,
        "workspacePath": "",
        "agentType": descriptor.agent_type,
        "prompt": request.prompt,
        "approvalPolicy": DELEGATED_APPROVAL_POLICY,
        "delegation": delegation_block(request),
    });
    if let Some(title) = request
        .title
        .as_ref()
        .filter(|title| !title.trim().is_empty())
    {
        payload["title"] = Value::String(title.clone());
    }
    payload
}

/// The target's refusal reads as the reason, so it is passed through.
fn target_error(error: anyhow::Error) -> PortError {
    PortError::new(PortErrorKind::Backend, format!("{error:#}"))
}

enum CatalogError {
    Incompatible(String),
    Unreachable(String),
}

fn has_capability(protocol: &Value, wanted: &str) -> bool {
    protocol
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .any(|capability| capability.as_str() == Some(wanted))
        })
}

/// Whether a probed target can take delegated jobs from this build.
fn ensure_delegation_target(protocol: &Value, device_name: &str) -> Result<(), CatalogError> {
    dispatch_ssh::validate_dispatch_protocol(protocol, Some(DELEGATED_APPROVAL_POLICY)).map_err(
        |error| CatalogError::Incompatible(format!("{device_name} needs an update: {error}")),
    )?;
    if has_capability(protocol, DISPATCH_AGENT_DELEGATION_CAPABILITY) {
        Ok(())
    } else {
        Err(CatalogError::Incompatible(format!(
            "{device_name} runs an OpenBitFun version without agent delegation"
        )))
    }
}

fn descriptors_for(device_id: &str, catalog: &DeviceCatalog) -> Vec<RemoteAgentDescriptor> {
    let availability = match &catalog.problem {
        None => RemoteAgentAvailability::Available,
        Some((reason, detail)) => RemoteAgentAvailability::Unavailable {
            reason: *reason,
            detail: detail.clone(),
        },
    };
    catalog
        .agents
        .iter()
        .filter_map(|agent| {
            let task_kinds: Vec<RemoteAgentTaskKind> = agent
                .task_kinds
                .iter()
                .filter_map(|kind| match kind.as_str() {
                    "message" => Some(RemoteAgentTaskKind::Message),
                    "coding" => Some(RemoteAgentTaskKind::Coding),
                    _ => None,
                })
                .collect();
            // An entry only a newer build understands cannot be delegated to.
            if task_kinds.is_empty() {
                return None;
            }
            Some(RemoteAgentDescriptor {
                address: RemoteAgentAddress {
                    device_id: device_id.to_string(),
                    open_agent_id: agent.id.clone(),
                },
                device_name: catalog.device_name.clone(),
                agent_type: agent.agent_type.clone(),
                display_name: if agent.display_name.trim().is_empty() {
                    agent.id.clone()
                } else {
                    agent.display_name.clone()
                },
                description: agent.description.clone(),
                task_kinds,
                availability: availability.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct Devices(Mutex<Vec<AccountDevice>>);

    #[async_trait]
    impl AccountDeviceDirectory for Devices {
        async fn other_devices(&self) -> anyhow::Result<Vec<AccountDevice>> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    /// Answers probe and catalog per device id.
    struct Targets {
        probes: HashMap<&'static str, Value>,
        catalogs: HashMap<&'static str, Value>,
        catalog_calls: AtomicUsize,
        /// Job commands this device received.
        job_calls: Mutex<Vec<(String, Value)>>,
    }

    impl Targets {
        fn new(
            probes: HashMap<&'static str, Value>,
            catalogs: HashMap<&'static str, Value>,
        ) -> Self {
            Self {
                probes,
                catalogs,
                catalog_calls: AtomicUsize::new(0),
                job_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl DeviceDispatchRpc for Targets {
        async fn invoke(
            &self,
            device_id: &str,
            command: &str,
            args: Value,
        ) -> anyhow::Result<Value> {
            if command.starts_with("dispatch_target_")
                && command != "dispatch_target_probe"
                && command != "dispatch_target_catalog"
            {
                self.job_calls
                    .lock()
                    .unwrap()
                    .push((command.to_string(), args.clone()));
                return Ok(json!({
                    "accepted": true,
                    "jobId": args["jobId"],
                    "sessionId": args["sessionId"],
                    "cancelled": true,
                }));
            }
            assert_eq!(args, json!({}), "the requester never names itself");
            let table = match command {
                "dispatch_target_probe" => &self.probes,
                "dispatch_target_catalog" => {
                    self.catalog_calls.fetch_add(1, Ordering::SeqCst);
                    &self.catalogs
                }
                other => panic!("unexpected command {other}"),
            };
            table
                .get(device_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("{device_id} rejected {command}"))
        }
    }

    fn probe(capabilities: &[&str]) -> Value {
        let mut all: Vec<&str> =
            openbitfun_services_core::dispatch_contract::dispatch_required_target_capabilities()
                .collect();
        all.extend_from_slice(capabilities);
        json!({
            "productId": openbitfun_services_core::product_identity::product_id(),
            "dataNamespace": openbitfun_services_core::product_identity::data_namespace(),
            "protocolVersion": openbitfun_services_core::dispatch_contract::DISPATCH_PROTOCOL_VERSION,
            "capabilities": all,
        })
    }

    fn catalog(id: &str, kinds: &[&str]) -> Value {
        json!({ "agents": [{
            "id": id, "agentType": "Standard", "displayName": "Helper", "taskKinds": kinds
        }]})
    }

    fn device(id: &str, online: bool) -> AccountDevice {
        AccountDevice {
            device_id: id.to_string(),
            device_name: id.to_uppercase(),
            online,
        }
    }

    fn directory(
        targets: Targets,
        devices: Vec<AccountDevice>,
    ) -> (RemoteAgentDirectory, Arc<Devices>, Arc<Targets>) {
        let devices = Arc::new(Devices(Mutex::new(devices)));
        let targets = Arc::new(targets);
        (
            RemoteAgentDirectory::new(targets.clone(), devices.clone()),
            devices,
            targets,
        )
    }

    #[tokio::test]
    async fn lists_only_delegating_devices_and_names_the_reason_otherwise() {
        let targets = Targets {
            probes: HashMap::from([
                ("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY])),
                ("old", probe(&[])),
            ]),
            catalogs: HashMap::from([
                ("laptop", catalog("helper", &["message", "telepathy"])),
                ("old", catalog("never-asked", &["message"])),
            ]),
            catalog_calls: AtomicUsize::new(0),
            job_calls: Mutex::new(Vec::new()),
        };
        let (directory, _, targets) = directory(
            targets,
            vec![
                device("laptop", true),
                device("old", true),
                device("phone", false),
            ],
        );
        let listed = directory.list(RemoteAgentListing::Refreshed).await;
        assert_eq!(listed.len(), 1, "{listed:?}");
        let helper = &listed[0];
        assert_eq!(helper.address.agent_type(), "remote:laptop/helper");
        assert_eq!(helper.device_name, "LAPTOP");
        assert_eq!(helper.agent_type, "Standard");
        assert_eq!(helper.task_kinds, vec![RemoteAgentTaskKind::Message]);
        assert!(helper.availability.is_available());
        // The incompatible device is never asked for its catalog.
        assert_eq!(targets.catalog_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn external_agents_are_listed_only_from_targets_that_run_them() {
        let both = json!({ "agents": [
            { "id": "helper", "agentType": "Standard", "displayName": "Helper", "taskKinds": ["message"] },
            { "id": "codex", "agentType": "acp:codex", "displayName": "Codex", "taskKinds": ["message", "coding"] },
        ]});
        let targets = Targets {
            probes: HashMap::from([
                ("old", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY])),
                (
                    "new",
                    probe(&[
                        DISPATCH_AGENT_DELEGATION_CAPABILITY,
                        DISPATCH_EXTERNAL_AGENT_EXECUTOR_CAPABILITY,
                    ]),
                ),
            ]),
            catalogs: HashMap::from([("old", both.clone()), ("new", both)]),
            catalog_calls: AtomicUsize::new(0),
            job_calls: Mutex::new(Vec::new()),
        };
        let (directory, _, _) = directory(targets, vec![device("old", true), device("new", true)]);
        let mut listed: Vec<String> = directory
            .list(RemoteAgentListing::Refreshed)
            .await
            .iter()
            .map(|agent| agent.address.agent_type())
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            vec![
                "remote:new/codex".to_string(),
                "remote:new/helper".to_string(),
                "remote:old/helper".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn cached_listings_reuse_answers_and_keep_offline_agents_with_a_reason() {
        let targets = Targets {
            probes: HashMap::from([("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY]))]),
            catalogs: HashMap::from([("laptop", catalog("helper", &["message", "coding"]))]),
            catalog_calls: AtomicUsize::new(0),
            job_calls: Mutex::new(Vec::new()),
        };
        let (directory, devices, targets) = directory(targets, vec![device("laptop", true)]);
        assert_eq!(directory.list(RemoteAgentListing::Cached).await.len(), 1);
        assert_eq!(directory.list(RemoteAgentListing::Cached).await.len(), 1);
        assert_eq!(targets.catalog_calls.load(Ordering::SeqCst), 1);
        directory.list(RemoteAgentListing::Refreshed).await;
        assert_eq!(targets.catalog_calls.load(Ordering::SeqCst), 2);

        *devices.0.lock().unwrap() = vec![device("laptop", false)];
        let offline = directory.list(RemoteAgentListing::Cached).await;
        assert_eq!(offline.len(), 1);
        assert_eq!(
            offline[0].availability,
            RemoteAgentAvailability::Unavailable {
                reason: RemoteAgentUnavailableReason::Offline,
                detail: "LAPTOP is offline".to_string(),
            }
        );
        let address = offline[0].address.clone();
        assert!(!directory
            .find(&address)
            .await
            .expect("still listed")
            .availability
            .is_available());
    }

    #[tokio::test]
    async fn unreachable_devices_report_the_error() {
        let targets = Targets {
            probes: HashMap::from([("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY]))]),
            catalogs: HashMap::new(),
            catalog_calls: AtomicUsize::new(0),
            job_calls: Mutex::new(Vec::new()),
        };
        let (directory, _, _) = directory(targets, vec![device("laptop", true)]);
        // Nothing was ever listed, so there is no agent to report.
        assert!(directory
            .list(RemoteAgentListing::Refreshed)
            .await
            .is_empty());
    }

    fn submit_request(
        open_agent_id: &str,
        task_kind: RemoteAgentTaskKind,
    ) -> openbitfun_runtime_ports::RemoteAgentSubmitRequest {
        openbitfun_runtime_ports::RemoteAgentSubmitRequest {
            address: RemoteAgentAddress {
                device_id: "laptop".to_string(),
                open_agent_id: open_agent_id.to_string(),
            },
            job_id: "delegation-1".to_string(),
            session_id: "job-session".to_string(),
            task_kind,
            prompt: "Summarize the notes".to_string(),
            title: Some("Notes".to_string()),
            source: openbitfun_runtime_ports::RemoteAgentSource {
                session_id: "parent".to_string(),
                agent_type: "Standard".to_string(),
                display_name: None,
            },
            source_workspace_path: Some("/local/workspace".to_string()),
        }
    }

    #[tokio::test]
    async fn submissions_name_the_open_agent_and_never_the_local_workspace() {
        use openbitfun_runtime_ports::RemoteAgentDelegationPort;
        let targets = Targets::new(
            HashMap::from([("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY]))]),
            HashMap::from([("laptop", catalog("helper", &["message"]))]),
        );
        let (directory, _, targets) = directory(targets, vec![device("laptop", true)]);

        let job = directory
            .submit(submit_request("helper", RemoteAgentTaskKind::Message))
            .await
            .expect("submit");
        assert_eq!(job.job_id, "delegation-1");
        let calls = targets.job_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        let (command, payload) = &calls[0];
        assert_eq!(command, "dispatch_target_submit");
        assert_eq!(payload["agentType"], "Standard");
        assert_eq!(payload["workspacePath"], "");
        assert_eq!(payload["approvalPolicy"], "remote");
        assert_eq!(payload["delegation"]["openAgentId"], "helper");
        assert_eq!(payload["delegation"]["taskKind"], "message");
        assert_eq!(payload["delegation"]["source"]["sessionId"], "parent");
        assert_eq!(payload["title"], "Notes");
        assert!(
            !payload.to_string().contains("/local/workspace"),
            "a message job never carries a local path"
        );

        directory
            .answer(
                &job,
                "target-1",
                openbitfun_runtime_ports::PermissionReply::Once,
            )
            .await
            .expect("answer");
        let calls = targets.job_calls.lock().unwrap().clone();
        assert_eq!(calls[1].0, "dispatch_target_answer");
        assert_eq!(calls[1].1["jobId"], "delegation-1");
        assert_eq!(calls[1].1["requestId"], "target-1");
        assert!(directory.cancel(&job).await.expect("cancel"));
    }

    #[tokio::test]
    async fn refused_submissions_never_reach_the_device() {
        use openbitfun_runtime_ports::RemoteAgentDelegationPort;
        let targets = Targets::new(
            HashMap::from([("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY]))]),
            HashMap::from([("laptop", catalog("helper", &["message", "coding"]))]),
        );
        let (directory, _, targets) = directory(targets, vec![device("laptop", true)]);

        let unknown = directory
            .submit(submit_request("writer", RemoteAgentTaskKind::Message))
            .await
            .unwrap_err();
        assert_eq!(unknown.kind, PortErrorKind::NotFound);
        let coding = directory
            .submit(submit_request("helper", RemoteAgentTaskKind::Coding))
            .await
            .unwrap_err();
        assert_eq!(coding.kind, PortErrorKind::NotAvailable);
        assert!(targets.job_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn coding_tasks_need_a_local_git_workspace_before_the_device_is_contacted() {
        use openbitfun_runtime_ports::RemoteAgentDelegationPort;
        let targets = || {
            Targets::new(
                HashMap::from([("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY]))]),
                HashMap::from([("laptop", catalog("helper", &["message", "coding"]))]),
            )
        };
        let (without_store, _, devices) = directory(targets(), vec![device("laptop", true)]);
        let error = without_store
            .submit(submit_request("helper", RemoteAgentTaskKind::Coding))
            .await
            .unwrap_err();
        assert_eq!(error.kind, PortErrorKind::NotAvailable, "{}", error.message);
        assert!(devices.job_calls.lock().unwrap().is_empty());

        let temp = tempfile::tempdir().expect("tempdir");
        let (with_store, _, devices) = directory(targets(), vec![device("laptop", true)]);
        let directory = with_store.with_outbound_store(
            OutboundDispatchStore::new_in_root_for_tests(temp.path().join("outbound")),
        );
        let not_git = temp.path().join("notes");
        std::fs::create_dir_all(&not_git).expect("workspace");
        let mut request = submit_request("helper", RemoteAgentTaskKind::Coding);
        request.source_workspace_path = Some(not_git.to_string_lossy().to_string());
        let error = directory.submit(request).await.unwrap_err();
        assert_eq!(error.kind, PortErrorKind::InvalidRequest);
        assert!(
            error.message.contains("Git workspace on this device"),
            "{}",
            error.message
        );

        let mut request = submit_request("helper", RemoteAgentTaskKind::Coding);
        request.source_workspace_path = None;
        let error = directory.submit(request).await.unwrap_err();
        assert_eq!(error.kind, PortErrorKind::InvalidRequest);
        assert!(devices.job_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn sync_answers_become_coding_results() {
        let changed = json!({
            "changed": true,
            "branch": "openbitfun/dispatch/delegation-1",
            "baseCommit": "0".repeat(40),
            "headCommit": "1".repeat(40),
            "syncedHeadCommit": "2".repeat(40),
            "commitCount": 1,
            "changes": [{ "status": "A", "path": "new.txt" }],
            "truncatedChanges": false,
            "bundleSize": 10,
            "baselineWorktreePath": "/w/delegation-1",
        });
        let result = coding_result(&changed, Some("/w/delegation-1".to_string())).unwrap();
        assert_eq!(result.head_commit, "2".repeat(40), "the synced head wins");
        assert_eq!(result.changes[0].path, "new.txt");
        assert_eq!(result.worktree_path.as_deref(), Some("/w/delegation-1"));

        let unchanged = json!({
            "changed": false,
            "branch": "openbitfun/dispatch/delegation-1",
            "baseCommit": "0".repeat(40),
            "headCommit": "0".repeat(40),
            "commitCount": 0,
            "changes": [],
            "truncatedChanges": false,
            "bundleSize": 0,
        });
        let result = coding_result(&unchanged, None).unwrap();
        assert_eq!(result.commit_count, 0);
        assert!(result.changes.is_empty());

        let error = coding_result(&json!({ "changed": true }), None).unwrap_err();
        assert_eq!(error.kind, PortErrorKind::Backend);
    }

    #[test]
    fn delegated_jobs_stay_out_of_the_dispatch_panel() {
        assert!(!super::super::controller::listed_in_dispatch_panel(
            "delegation-1"
        ));
        assert!(super::super::controller::listed_in_dispatch_panel(
            "dispatch-1a2b"
        ));
    }
}
