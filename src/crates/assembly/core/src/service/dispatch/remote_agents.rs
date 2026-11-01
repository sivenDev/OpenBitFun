//! Requester side of agent delegation: the agents other devices of this
//! account opened to this device, reached through account-device dispatch,
//! and the agents paired devices of other accounts granted to this device,
//! reached over their cross-account connection.
//!
//! Hosts supply the device RPC and the account's device list, and optionally
//! the connection RPC and the usable connections; this module decides which
//! targets can take delegated work and what they offer. A target is queried
//! only when it is online and advertises agent delegation plus the dispatch
//! surface this build requires, so an older device is reported as
//! incompatible instead of failing later. A connection target must also
//! advertise cross-account delegation.
//!
//! A connection target's id is `conn:<connectionId>`; every call to it
//! travels over the connection.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use openbitfun_runtime_ports::{
    remote_connection_target, remote_target_connection, PortError, PortErrorKind, PortResult,
    RemoteAgentAccount, RemoteAgentAddress, RemoteAgentAvailability, RemoteAgentDescriptor,
    RemoteAgentListing, RemoteAgentUnavailableReason,
};
use openbitfun_services_core::dispatch_contract::{
    DispatchCatalogAgent, DispatchCatalogResponse, DISPATCH_AGENT_DELEGATION_CAPABILITY,
    DISPATCH_CROSS_ACCOUNT_DELEGATION_CAPABILITY,
};
use openbitfun_services_integrations::remote_ssh::dispatch_ssh;
use serde_json::{json, Value};

use super::device_controller::DeviceDispatchRpc;

/// How long a listing may reuse a device's last answer.
const CATALOG_CACHE_TTL: Duration = Duration::from_secs(30);
/// Bound on probing and listing one device, so a slow device cannot stall
/// every turn's catalog.
const DEVICE_QUERY_TIMEOUT: Duration = Duration::from_secs(8);
/// Delegated jobs always ask the target owner for approval.
const DELEGATED_APPROVAL_POLICY: &str = "remote";
/// The only task kind a delegated job carries; targets still read the field.
const DELEGATED_TASK_KIND: &str = "message";

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

/// A usable cross-account connection: the peer device and its account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionPeer {
    pub connection_id: String,
    pub peer_user_id: String,
    pub peer_display_name: String,
    pub peer_device_name: String,
}

/// The host's cross-account connections and their encrypted RPC.
#[async_trait]
pub trait ConnectionDispatchRpc: Send + Sync {
    /// Connections that can carry calls now: confirmed, not revoked, and
    /// with a verified peer key.
    async fn usable_connections(&self) -> anyhow::Result<Vec<ConnectionPeer>>;

    /// Calls a target command on the peer device of `connection_id`.
    async fn invoke_connection(
        &self,
        connection_id: &str,
        command: &str,
        args: Value,
    ) -> anyhow::Result<Value>;

    /// The refusal `error` carries when its connection is gone for good:
    /// revoked, removed, or no longer trusted. Anything else may pass.
    fn gone_reason(&self, _error: &anyhow::Error) -> Option<GoneConnection> {
        None
    }
}

/// Why a connection will not carry calls again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoneConnection {
    Revoked,
    KeyChanged,
}

/// Sends calls for `conn:` targets over their connection and every other call
/// to the account device RPC, so every job call shares one path for both
/// kinds of target.
struct RoutedDispatchRpc {
    devices: Arc<dyn DeviceDispatchRpc>,
    connections: Arc<dyn ConnectionDispatchRpc>,
}

#[async_trait]
impl DeviceDispatchRpc for RoutedDispatchRpc {
    async fn invoke(&self, device_id: &str, command: &str, args: Value) -> anyhow::Result<Value> {
        match remote_target_connection(device_id) {
            Some(connection_id) => {
                self.connections
                    .invoke_connection(connection_id, command, args)
                    .await
            }
            None => self.devices.invoke(device_id, command, args).await,
        }
    }
}

/// One target a listing queries: an account device or a connection peer.
#[derive(Debug, Clone)]
struct ListedTarget {
    device: AccountDevice,
    account: Option<RemoteAgentAccount>,
}

#[derive(Clone)]
struct DeviceCatalog {
    fetched_at: Instant,
    device_name: String,
    /// The device's last successful answer, kept so its agents stay listed
    /// with a reason while it is offline or unreachable.
    agents: Vec<DispatchCatalogAgent>,
    problem: Option<(RemoteAgentUnavailableReason, String)>,
    account: Option<RemoteAgentAccount>,
}

/// Lists and caches the agents other devices opened to this device.
pub struct RemoteAgentDirectory {
    rpc: Arc<dyn DeviceDispatchRpc>,
    devices: Arc<dyn AccountDeviceDirectory>,
    connections: Option<Arc<dyn ConnectionDispatchRpc>>,
    cache: tokio::sync::Mutex<HashMap<String, DeviceCatalog>>,
}

impl RemoteAgentDirectory {
    pub fn new(rpc: Arc<dyn DeviceDispatchRpc>, devices: Arc<dyn AccountDeviceDirectory>) -> Self {
        Self {
            rpc,
            devices,
            connections: None,
            cache: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Why a job of another account can no longer be reached, when its
    /// connection was revoked, removed, or stopped being trusted.
    fn gone_connection_reason(&self, device_id: &str, error: &anyhow::Error) -> Option<String> {
        remote_target_connection(device_id)?;
        match self.connections.as_ref()?.gone_reason(error)? {
            GoneConnection::Revoked => Some(
                "Access was revoked: the connection to the other account's device was revoked, \
                 and the task there was stopped"
                    .to_string(),
            ),
            GoneConnection::KeyChanged => Some(
                "The other account's device changed its key, so this device stopped trusting \
                 the connection; pair again to delegate to it"
                    .to_string(),
            ),
        }
    }

    /// Adds the agents paired devices of other accounts granted to this
    /// device. Calls to them travel over their connection.
    pub fn with_connections(mut self, connections: Arc<dyn ConnectionDispatchRpc>) -> Self {
        self.rpc = Arc::new(RoutedDispatchRpc {
            devices: self.rpc.clone(),
            connections: connections.clone(),
        });
        self.connections = Some(connections);
        self
    }

    /// Account devices, then usable connection peers. Either source failing
    /// leaves the other listed.
    async fn targets(&self) -> Vec<ListedTarget> {
        let mut targets: Vec<ListedTarget> = match self.devices.other_devices().await {
            Ok(devices) => devices
                .into_iter()
                .filter(|device| remote_target_connection(&device.device_id).is_none())
                .map(|device| ListedTarget {
                    device,
                    account: None,
                })
                .collect(),
            Err(error) => {
                log::debug!("Remote agents not listed: account devices unavailable: {error}");
                Vec::new()
            }
        };
        if let Some(connections) = &self.connections {
            match connections.usable_connections().await {
                Ok(peers) => targets.extend(peers.into_iter().map(|peer| ListedTarget {
                    device: AccountDevice {
                        device_id: remote_connection_target(&peer.connection_id),
                        device_name: peer.peer_device_name,
                        // Presence across accounts is not shared; an
                        // unreachable peer is reported when it does not answer.
                        online: true,
                    },
                    account: Some(RemoteAgentAccount {
                        user_id: peer.peer_user_id,
                        display_name: peer.peer_display_name,
                    }),
                })),
                Err(error) => {
                    log::debug!("Connection agents not listed: connections unavailable: {error}");
                }
            }
        }
        targets
    }

    pub async fn list(&self, listing: RemoteAgentListing) -> Vec<RemoteAgentDescriptor> {
        let targets = self.targets().await;
        let previous = self.cache.lock().await.clone();
        let catalogs = futures::future::join_all(targets.into_iter().map(|target| {
            let previous = previous.get(&target.device.device_id).cloned();
            async move {
                let ListedTarget { device, account } = target;
                let catalog = match previous {
                    Some(cached)
                        if listing == RemoteAgentListing::Cached
                            && device.online
                            && cached.problem.is_none()
                            && cached.fetched_at.elapsed() < CATALOG_CACHE_TTL =>
                    {
                        cached
                    }
                    previous => self.query_device(&device, account, previous).await,
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
        account: Option<RemoteAgentAccount>,
        previous: Option<DeviceCatalog>,
    ) -> DeviceCatalog {
        let known_agents = previous.map(|catalog| catalog.agents).unwrap_or_default();
        let unavailable = |reason, detail: String| DeviceCatalog {
            fetched_at: Instant::now(),
            device_name: device.device_name.clone(),
            agents: known_agents.clone(),
            problem: Some((reason, detail)),
            account: account.clone(),
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
                account: account.clone(),
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
        if remote_target_connection(&device.device_id).is_some()
            && !has_capability(&protocol, DISPATCH_CROSS_ACCOUNT_DELEGATION_CAPABILITY)
        {
            return Err(CatalogError::Incompatible(format!(
                "{} runs an OpenBitFun version without cross-account delegation",
                device.device_name
            )));
        }
        let catalog = self
            .rpc
            .invoke(&device.device_id, "dispatch_target_catalog", json!({}))
            .await
            .map_err(|error| CatalogError::Unreachable(format!("{error:#}")))?;
        serde_json::from_value::<DispatchCatalogResponse>(catalog)
            // Only assistants are opened; an older target may still list
            // agent modes or external agents, which are not delegated to.
            .map(|catalog| {
                catalog
                    .agents
                    .into_iter()
                    .filter(|agent| {
                        crate::service::delegation::assistant_directory(&agent.agent_type).is_some()
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
                // A connection that is gone never comes back, so the job ends
                // here instead of being observed again forever.
                if let Some(reason) = self.gone_connection_reason(&job.device_id, &error) {
                    return Ok(openbitfun_runtime_ports::RemoteAgentJobSnapshot {
                        state: openbitfun_runtime_ports::RemoteAgentJobState::Failed,
                        cursor,
                        events: Vec::new(),
                        pending_permissions: Vec::new(),
                        cursor_reset: false,
                        last_error: Some(reason),
                        hold: None,
                    });
                }
                return Err(target_error(error));
            }
        };
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

/// Names the open agent and the requesting session for the target's
/// admission and audit.
fn delegation_block(request: &openbitfun_runtime_ports::RemoteAgentSubmitRequest) -> Value {
    let mut source = json!({
        "sessionId": request.source.session_id,
        "agentType": request.source.agent_type,
    });
    if let Some(display_name) = &request.source.display_name {
        source["displayName"] = Value::String(display_name.clone());
    }
    json!({
        "openAgentId": request.address.open_agent_id,
        "taskKind": DELEGATED_TASK_KIND,
        "source": source,
    })
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
            // Delegated jobs are message tasks; an entry that does not take
            // them cannot be delegated to.
            if !agent
                .task_kinds
                .iter()
                .any(|kind| kind == DELEGATED_TASK_KIND)
            {
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
                account: catalog.account.clone(),
                availability: match (&availability, &agent.unavailable) {
                    (RemoteAgentAvailability::Available, Some(detail)) => {
                        RemoteAgentAvailability::Unavailable {
                            reason: RemoteAgentUnavailableReason::AgentUnavailable,
                            detail: detail.clone(),
                        }
                    }
                    _ => availability.clone(),
                },
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
            "id": id, "agentType": "assistant:workspace", "displayName": "Helper", "taskKinds": kinds
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
        assert_eq!(helper.agent_type, "assistant:workspace");
        assert!(helper.availability.is_available());
        // The incompatible device is never asked for its catalog.
        assert_eq!(targets.catalog_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn only_assistants_are_listed_and_unavailable_ones_say_why() {
        let older = json!({ "agents": [
            { "id": "helper", "agentType": "assistant:workspace", "displayName": "Helper", "taskKinds": ["message"] },
            { "id": "coder", "agentType": "assistant:workspace-coder", "displayName": "Coder",
              "taskKinds": ["message"], "unavailable": "the assistant was removed" },
            { "id": "mode", "agentType": "Standard", "displayName": "Mode", "taskKinds": ["message"] },
            { "id": "codex", "agentType": "acp:codex", "displayName": "Codex", "taskKinds": ["message", "coding"] },
        ]});
        let targets = Targets::new(
            HashMap::from([("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY]))]),
            HashMap::from([("laptop", older)]),
        );
        let (directory, _, _) = directory(targets, vec![device("laptop", true)]);
        let agents = directory.list(RemoteAgentListing::Refreshed).await;
        let mut listed: Vec<String> = agents
            .iter()
            .map(|agent| agent.address.agent_type())
            .collect();
        listed.sort();
        assert_eq!(listed, vec!["remote:laptop/coder", "remote:laptop/helper"]);
        let coder = agents
            .iter()
            .find(|agent| agent.address.open_agent_id == "coder")
            .unwrap();
        assert_eq!(
            coder.availability,
            RemoteAgentAvailability::Unavailable {
                reason: RemoteAgentUnavailableReason::AgentUnavailable,
                detail: "the assistant was removed".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn cached_listings_reuse_answers_and_keep_offline_agents_with_a_reason() {
        let targets = Targets {
            probes: HashMap::from([("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY]))]),
            catalogs: HashMap::from([("laptop", catalog("helper", &["message"]))]),
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

    fn submit_request(open_agent_id: &str) -> openbitfun_runtime_ports::RemoteAgentSubmitRequest {
        openbitfun_runtime_ports::RemoteAgentSubmitRequest {
            address: RemoteAgentAddress {
                device_id: "laptop".to_string(),
                open_agent_id: open_agent_id.to_string(),
            },
            job_id: "delegation-1".to_string(),
            session_id: "job-session".to_string(),
            prompt: "Summarize the notes".to_string(),
            title: Some("Notes".to_string()),
            source: openbitfun_runtime_ports::RemoteAgentSource {
                session_id: "parent".to_string(),
                agent_type: "Standard".to_string(),
                display_name: None,
            },
        }
    }

    /// Connection peers answered by their own target table, keyed by
    /// connection id.
    struct Connections {
        peers: Vec<ConnectionPeer>,
        targets: Targets,
    }

    #[async_trait]
    impl ConnectionDispatchRpc for Connections {
        async fn usable_connections(&self) -> anyhow::Result<Vec<ConnectionPeer>> {
            Ok(self.peers.clone())
        }

        async fn invoke_connection(
            &self,
            connection_id: &str,
            command: &str,
            args: Value,
        ) -> anyhow::Result<Value> {
            self.targets.invoke(connection_id, command, args).await
        }
    }

    fn peer(connection_id: &str) -> ConnectionPeer {
        ConnectionPeer {
            connection_id: connection_id.to_string(),
            peer_user_id: "2002".to_string(),
            peer_display_name: "Bea".to_string(),
            peer_device_name: "B1 Desktop".to_string(),
        }
    }

    struct Unsigned;

    #[async_trait]
    impl AccountDeviceDirectory for Unsigned {
        async fn other_devices(&self) -> anyhow::Result<Vec<AccountDevice>> {
            anyhow::bail!("the account relay cannot be reached")
        }
    }

    #[tokio::test]
    async fn connection_agents_are_listed_with_their_account_and_reached_over_the_connection() {
        let delegation = &[DISPATCH_AGENT_DELEGATION_CAPABILITY];
        let cross_account = &[
            DISPATCH_AGENT_DELEGATION_CAPABILITY,
            DISPATCH_CROSS_ACCOUNT_DELEGATION_CAPABILITY,
        ];
        let devices = Targets::new(
            HashMap::from([("laptop", probe(delegation))]),
            HashMap::from([("laptop", catalog("helper", &["message"]))]),
        );
        let connections = Arc::new(Connections {
            peers: vec![peer("c-new"), peer("c-old")],
            targets: Targets::new(
                HashMap::from([
                    ("c-new", probe(cross_account)),
                    ("c-old", probe(delegation)),
                ]),
                HashMap::from([
                    ("c-new", catalog("helper", &["message", "coding"])),
                    ("c-old", catalog("never-asked", &["message"])),
                ]),
            ),
        });
        let (directory, _, devices) = directory(devices, vec![device("laptop", true)]);
        let directory = directory.with_connections(connections.clone());

        let listed = directory.list(RemoteAgentListing::Refreshed).await;
        let types: Vec<String> = listed
            .iter()
            .map(|agent| agent.address.agent_type())
            .collect();
        // The same open agent id on another account is a separate entry; a
        // target without cross-account delegation lists nothing.
        assert_eq!(types, ["remote:laptop/helper", "remote:conn:c-new/helper"]);
        assert_eq!(listed[0].account, None);
        let other = &listed[1];
        assert_eq!(other.device_name, "B1 Desktop");
        assert_eq!(
            other.account,
            Some(RemoteAgentAccount {
                user_id: "2002".to_string(),
                display_name: "Bea".to_string(),
            })
        );
        assert_eq!(connections.targets.catalog_calls.load(Ordering::SeqCst), 1);

        // Submitting, observing, and cancelling travel over the connection.
        let mut request = submit_request("helper");
        request.address.device_id = remote_connection_target("c-new");
        let port: &dyn openbitfun_runtime_ports::RemoteAgentDelegationPort = &directory;
        let job = port
            .submit(request)
            .await
            .expect("submit over the connection");
        assert_eq!(job.device_id, "conn:c-new");
        assert!(port.cancel(&job).await.unwrap());
        let calls = connections.targets.job_calls.lock().unwrap().clone();
        assert_eq!(
            calls
                .iter()
                .map(|(command, _)| command.as_str())
                .collect::<Vec<_>>(),
            ["dispatch_target_submit", "dispatch_target_cancel"]
        );
        assert_eq!(calls[0].1["delegation"]["openAgentId"], "helper");
        assert_eq!(calls[0].1["approvalPolicy"], "remote");
        assert!(devices.job_calls.lock().unwrap().is_empty());
    }

    /// A connection the Relay refuses with `reason`.
    struct Refused(&'static str);

    #[async_trait]
    impl ConnectionDispatchRpc for Refused {
        async fn usable_connections(&self) -> anyhow::Result<Vec<ConnectionPeer>> {
            Ok(Vec::new())
        }

        async fn invoke_connection(
            &self,
            _connection_id: &str,
            _command: &str,
            _args: Value,
        ) -> anyhow::Result<Value> {
            anyhow::bail!("refused: {}", self.0)
        }

        fn gone_reason(&self, _error: &anyhow::Error) -> Option<GoneConnection> {
            match self.0 {
                "connection_revoked" => Some(GoneConnection::Revoked),
                "key_changed" => Some(GoneConnection::KeyChanged),
                _ => None,
            }
        }
    }

    #[tokio::test]
    async fn jobs_end_when_their_connection_is_gone_and_retry_otherwise() {
        let job = openbitfun_runtime_ports::RemoteAgentJob {
            device_id: remote_connection_target("c-1"),
            job_id: "delegation-1".to_string(),
            session_id: "s".to_string(),
        };
        for (reason, gone) in [
            ("connection_revoked", true),
            ("key_changed", true),
            ("relay_unavailable", false),
        ] {
            let (directory, _, _) =
                directory(Targets::new(HashMap::new(), HashMap::new()), Vec::new());
            let directory = directory.with_connections(Arc::new(Refused(reason)));
            let port: &dyn openbitfun_runtime_ports::RemoteAgentDelegationPort = &directory;
            let observed = port.observe(&job, 7).await;
            if gone {
                let snapshot = observed.expect("a gone connection ends the job");
                assert_eq!(
                    snapshot.state,
                    openbitfun_runtime_ports::RemoteAgentJobState::Failed
                );
                assert_eq!(snapshot.cursor, 7);
                assert!(snapshot.last_error.is_some(), "{reason}");
            } else {
                // Anything else may pass, so observation is retried.
                assert!(observed.is_err(), "{reason}");
            }
        }
    }

    #[tokio::test]
    async fn connection_agents_stay_listed_when_account_devices_cannot_be_read() {
        let connections = Arc::new(Connections {
            peers: vec![peer("c-new")],
            targets: Targets::new(
                HashMap::from([(
                    "c-new",
                    probe(&[
                        DISPATCH_AGENT_DELEGATION_CAPABILITY,
                        DISPATCH_CROSS_ACCOUNT_DELEGATION_CAPABILITY,
                    ]),
                )]),
                HashMap::from([("c-new", catalog("helper", &["message"]))]),
            ),
        });
        let directory = RemoteAgentDirectory::new(
            Arc::new(Targets::new(HashMap::new(), HashMap::new())),
            Arc::new(Unsigned),
        )
        .with_connections(connections);
        let listed = directory.list(RemoteAgentListing::Refreshed).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].address.agent_type(), "remote:conn:c-new/helper");
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
            .submit(submit_request("helper"))
            .await
            .expect("submit");
        assert_eq!(job.job_id, "delegation-1");
        let calls = targets.job_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        let (command, payload) = &calls[0];
        assert_eq!(command, "dispatch_target_submit");
        assert_eq!(payload["agentType"], "assistant:workspace");
        assert_eq!(payload["workspacePath"], "");
        assert_eq!(payload["approvalPolicy"], "remote");
        assert_eq!(payload["delegation"]["openAgentId"], "helper");
        assert_eq!(payload["delegation"]["taskKind"], "message");
        assert_eq!(payload["delegation"]["source"]["sessionId"], "parent");
        assert_eq!(payload["title"], "Notes");

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
            HashMap::from([("laptop", catalog("helper", &["message"]))]),
        );
        let (directory, _, targets) = directory(targets, vec![device("laptop", true)]);

        let unknown = directory
            .submit(submit_request("writer"))
            .await
            .unwrap_err();
        assert_eq!(unknown.kind, PortErrorKind::NotFound);
        assert!(targets.job_calls.lock().unwrap().is_empty());
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
