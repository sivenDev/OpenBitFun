//! Drives the desktop requester against simulated devices on this machine.
//!
//! `scripts/dev/sim-devices.mjs --check` starts a local Relay with two
//! accounts and real CLI target daemons, then runs this test with the
//! manifest that describes them. The requester here is the kernel's
//! `RemoteAgentDirectory` over this host's HostInvoke envelope; only the
//! signed-in session is replaced by the simulation's A1 session.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use openbitfun_core::service::dispatch::{
    AccountDevice, AccountDeviceDirectory, DeviceDispatchRpc, RemoteAgentDirectory,
};
use openbitfun_core::service::remote_connect::{AccountClient, AccountSession};
use openbitfun_runtime_ports::{
    PermissionReply, PortErrorKind, RemoteAgentAddress, RemoteAgentDelegationPort, RemoteAgentJob,
    RemoteAgentJobSnapshot, RemoteAgentJobState, RemoteAgentListing, RemoteAgentSource,
    RemoteAgentSubmitRequest, RemoteAgentTaskKind,
};
use serde_json::Value;

const JOB_TIMEOUT: Duration = Duration::from_secs(90);
const STREAM_START_MARKER: &str = "ACTIVE_TURN_STREAM_MARKER";

struct Manifest {
    relay_url: String,
    requester_device_id: String,
    session: AccountSession,
    open_agent_id: String,
    answers: String,
    holds: String,
    asks_approval: String,
    other_account: String,
}

impl Manifest {
    fn read() -> Self {
        let path = std::env::var_os("OPENBITFUN_SIM_MANIFEST")
            .expect("OPENBITFUN_SIM_MANIFEST names the running simulation");
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(path).expect("read simulation manifest"))
                .expect("decode simulation manifest");
        let text = |pointer: &str| {
            manifest
                .pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("manifest is missing {pointer}"))
                .to_string()
        };
        let secret = decode_secret(&text("/requester/deviceSecretHex"));
        Self {
            relay_url: text("/relayUrl"),
            requester_device_id: text("/requester/deviceId"),
            session: AccountSession::new(
                text("/requester/token"),
                text("/requester/userId"),
                secret,
            ),
            open_agent_id: text("/openAgentId"),
            answers: text("/devices/answers"),
            holds: text("/devices/holds"),
            asks_approval: text("/devices/asksApproval"),
            other_account: text("/devices/otherAccount"),
        }
    }

    fn address(&self, device_id: &str) -> RemoteAgentAddress {
        RemoteAgentAddress {
            device_id: device_id.to_string(),
            open_agent_id: self.open_agent_id.clone(),
        }
    }
}

fn decode_secret(hex: &str) -> [u8; 32] {
    let mut secret = [0_u8; 32];
    assert_eq!(hex.len(), 64, "device secret must be 32 bytes");
    for (index, byte) in secret.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex secret");
    }
    secret
}

/// This host's device RPC, signed in as the simulation's A1.
struct SimulatedAccountRpc {
    relay_url: String,
    session: AccountSession,
}

#[async_trait]
impl DeviceDispatchRpc for SimulatedAccountRpc {
    async fn invoke(&self, device_id: &str, command: &str, args: Value) -> anyhow::Result<Value> {
        crate::api::dispatch_api::invoke_device_dispatch(command, args, |command_json| async move {
            AccountClient::new()
                .device_rpc(&self.relay_url, &self.session, device_id, &command_json)
                .await
                .map_err(|error| format!("{error:#}"))
        })
        .await
    }
}

struct SimulatedAccountDevices {
    relay_url: String,
    session: AccountSession,
    own_device_id: String,
}

#[async_trait]
impl AccountDeviceDirectory for SimulatedAccountDevices {
    async fn other_devices(&self) -> anyhow::Result<Vec<AccountDevice>> {
        Ok(AccountClient::new()
            .list_devices(&self.relay_url, &self.session)
            .await?
            .into_iter()
            .filter(|device| device.device_id != self.own_device_id)
            .map(|device| AccountDevice {
                device_id: device.device_id,
                device_name: device.device_name,
                online: device.online,
            })
            .collect())
    }
}

fn submit_request(address: RemoteAgentAddress, prompt: &str) -> RemoteAgentSubmitRequest {
    RemoteAgentSubmitRequest {
        address,
        job_id: format!("delegation-{}", uuid::Uuid::new_v4()),
        session_id: uuid::Uuid::new_v4().to_string(),
        task_kind: RemoteAgentTaskKind::Message,
        prompt: prompt.to_string(),
        title: Some("Simulated delegation".to_string()),
        source: RemoteAgentSource {
            session_id: "requester-session".to_string(),
            agent_type: "agentic".to_string(),
            display_name: None,
        },
        source_workspace_path: None,
    }
}

async fn observe_until(
    port: &dyn RemoteAgentDelegationPort,
    job: &RemoteAgentJob,
    mut done: impl FnMut(&RemoteAgentJobSnapshot) -> bool,
) -> (RemoteAgentJobSnapshot, Vec<Value>) {
    let deadline = Instant::now() + JOB_TIMEOUT;
    let mut cursor = 0;
    let mut events = Vec::new();
    loop {
        let page = port.observe(job, cursor).await.expect("observe job");
        cursor = page.cursor;
        events.extend(page.events.iter().cloned());
        if done(&page) {
            return (page, events);
        }
        assert!(
            !page.state.is_terminal(),
            "job ended as {:?} before the expected state: {:?}",
            page.state,
            page.last_error
        );
        assert!(
            Instant::now() < deadline,
            "job did not reach the expected state"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn mentions(events: &[Value], text: &str) -> bool {
    events.iter().any(|event| event.to_string().contains(text))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run by scripts/dev/sim-devices.mjs --check"]
async fn desktop_requester_delegates_to_simulated_devices() {
    let manifest = Manifest::read();
    let directory = RemoteAgentDirectory::new(
        Arc::new(SimulatedAccountRpc {
            relay_url: manifest.relay_url.clone(),
            session: manifest.session.clone(),
        }),
        Arc::new(SimulatedAccountDevices {
            relay_url: manifest.relay_url.clone(),
            session: manifest.session.clone(),
            own_device_id: manifest.requester_device_id.clone(),
        }),
    );
    let port: &dyn RemoteAgentDelegationPort = &directory;

    // Catalog: the agents same-account devices opened to A1, as `remote:`
    // subagents; the other account's device is not among them.
    let listed = port.list_agents(RemoteAgentListing::Refreshed).await;
    let mut available: Vec<String> = listed
        .iter()
        .filter(|agent| agent.availability.is_available())
        .map(|agent| agent.address.agent_type())
        .collect();
    available.sort();
    let mut expected: Vec<String> = [&manifest.answers, &manifest.holds, &manifest.asks_approval]
        .into_iter()
        .map(|device| manifest.address(device).agent_type())
        .collect();
    expected.sort();
    assert_eq!(available, expected, "{listed:?}");
    let refused = port
        .submit(submit_request(
            manifest.address(&manifest.other_account),
            "Reach another account",
        ))
        .await
        .expect_err("another account's agent is not open to this device");
    assert_eq!(refused.kind, PortErrorKind::NotFound, "{refused:?}");

    // Message.
    let job = port
        .submit(submit_request(
            manifest.address(&manifest.answers),
            "Say hello",
        ))
        .await
        .expect("submit message job");
    let (finished, events) = observe_until(port, &job, |page| page.state.is_terminal()).await;
    assert_eq!(
        finished.state,
        RemoteAgentJobState::Succeeded,
        "{:?}",
        finished.last_error
    );
    assert!(mentions(&events, STREAM_START_MARKER), "{events:?}");

    // Cancel.
    let job = port
        .submit(submit_request(
            manifest.address(&manifest.holds),
            "Work until cancelled",
        ))
        .await
        .expect("submit long job");
    observe_until(port, &job, |page| {
        mentions(&page.events, STREAM_START_MARKER)
    })
    .await;
    assert!(port.cancel(&job).await.expect("cancel job"));
    let (cancelled, _) = observe_until(port, &job, |page| page.state.is_terminal()).await;
    assert_eq!(cancelled.state, RemoteAgentJobState::Cancelled);

    // Approval: answered from this device, carried out on the target.
    let job = port
        .submit(submit_request(
            manifest.address(&manifest.asks_approval),
            "Create the marker file",
        ))
        .await
        .expect("submit approval job");
    let (waiting, _) = observe_until(port, &job, |page| !page.pending_permissions.is_empty()).await;
    let prompt = &waiting.pending_permissions[0];
    assert_eq!(prompt.action, "bash", "{prompt:?}");
    port.answer(&job, &prompt.request_id, PermissionReply::Once)
        .await
        .expect("answer the target's prompt");
    let (approved, _) = observe_until(port, &job, |page| page.state.is_terminal()).await;
    assert_eq!(
        approved.state,
        RemoteAgentJobState::Succeeded,
        "{:?}",
        approved.last_error
    );
}
