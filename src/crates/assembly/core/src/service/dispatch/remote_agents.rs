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
    RemoteAgentDescriptor, RemoteAgentListing, RemoteAgentTaskKind, RemoteAgentUnavailableReason,
};
use openbitfun_services_core::dispatch_contract::{
    DispatchCatalogAgent, DispatchCatalogResponse, DISPATCH_AGENT_DELEGATION_CAPABILITY,
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
}

impl RemoteAgentDirectory {
    pub fn new(rpc: Arc<dyn DeviceDispatchRpc>, devices: Arc<dyn AccountDeviceDirectory>) -> Self {
        Self {
            rpc,
            devices,
            cache: tokio::sync::Mutex::new(HashMap::new()),
        }
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
        let catalog = self
            .rpc
            .invoke(&device.device_id, "dispatch_target_catalog", json!({}))
            .await
            .map_err(|error| CatalogError::Unreachable(format!("{error:#}")))?;
        serde_json::from_value::<DispatchCatalogResponse>(catalog)
            .map(|catalog| catalog.agents)
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
        Err(not_yet_supported(&request.address.agent_type()))
    }

    async fn observe(
        &self,
        job: &openbitfun_runtime_ports::RemoteAgentJob,
        _cursor: u64,
    ) -> PortResult<openbitfun_runtime_ports::RemoteAgentJobSnapshot> {
        Err(not_yet_supported(&job.job_id))
    }

    async fn cancel(&self, job: &openbitfun_runtime_ports::RemoteAgentJob) -> PortResult<bool> {
        Err(not_yet_supported(&job.job_id))
    }

    async fn answer(
        &self,
        job: &openbitfun_runtime_ports::RemoteAgentJob,
        _request_id: &str,
        _reply: openbitfun_runtime_ports::PermissionReply,
    ) -> PortResult<()> {
        Err(not_yet_supported(&job.job_id))
    }

    async fn continue_job(
        &self,
        job: &openbitfun_runtime_ports::RemoteAgentJob,
        _turn_id: &str,
        _prompt: String,
    ) -> PortResult<()> {
        Err(not_yet_supported(&job.job_id))
    }
}

fn not_yet_supported(subject: &str) -> PortError {
    PortError::new(
        PortErrorKind::NotAvailable,
        format!("delegating {subject} to another device is not supported by this build yet"),
    )
}

enum CatalogError {
    Incompatible(String),
    Unreachable(String),
}

/// Whether a probed target can take delegated jobs from this build.
fn ensure_delegation_target(protocol: &Value, device_name: &str) -> Result<(), CatalogError> {
    dispatch_ssh::validate_dispatch_protocol(protocol, Some(DELEGATED_APPROVAL_POLICY)).map_err(
        |error| CatalogError::Incompatible(format!("{device_name} needs an update: {error}")),
    )?;
    let delegates = protocol
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .any(|capability| capability.as_str() == Some(DISPATCH_AGENT_DELEGATION_CAPABILITY))
        });
    if delegates {
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
    }

    #[async_trait]
    impl DeviceDispatchRpc for Targets {
        async fn invoke(
            &self,
            device_id: &str,
            command: &str,
            args: Value,
        ) -> anyhow::Result<Value> {
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
    async fn cached_listings_reuse_answers_and_keep_offline_agents_with_a_reason() {
        let targets = Targets {
            probes: HashMap::from([("laptop", probe(&[DISPATCH_AGENT_DELEGATION_CAPABILITY]))]),
            catalogs: HashMap::from([("laptop", catalog("helper", &["message", "coding"]))]),
            catalog_calls: AtomicUsize::new(0),
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
        };
        let (directory, _, _) = directory(targets, vec![device("laptop", true)]);
        // Nothing was ever listed, so there is no agent to report.
        assert!(directory
            .list(RemoteAgentListing::Refreshed)
            .await
            .is_empty());
    }
}
