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
    RemoteAgentSubmitRequest,
};
use serde_json::Value;

const JOB_TIMEOUT: Duration = Duration::from_secs(90);
const STREAM_START_MARKER: &str = "ACTIVE_TURN_STREAM_MARKER";

struct Manifest {
    raw: Value,
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
            raw: manifest.clone(),
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
        prompt: prompt.to_string(),
        title: Some("Simulated delegation".to_string()),
        source: RemoteAgentSource {
            session_id: "requester-session".to_string(),
            agent_type: "agentic".to_string(),
            display_name: None,
        },
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

impl Manifest {
    fn text(&self, pointer: &str) -> String {
        self.raw
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("manifest is missing {pointer}"))
            .to_string()
    }

    /// Runs `openbitfun <args>` as the owner of `device_id`, with `input`
    /// on stdin; returns whether it succeeded and its output.
    fn owner_cli(&self, device_id: &str, args: &[&str], input: Option<&Value>) -> (bool, String) {
        use std::io::Write;
        let cli = self
            .raw
            .pointer(&format!("/deviceCli/{device_id}"))
            .unwrap_or_else(|| panic!("manifest has no CLI for {device_id}"));
        let environment = &cli["environment"];
        let mut command = openbitfun_services_core::process_manager::create_command(
            cli["binary"].as_str().expect("device CLI binary"),
        );
        command
            .args(args)
            .current_dir(environment["cwd"].as_str().expect("device CLI directory"))
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for name in environment["remove"].as_array().into_iter().flatten() {
            command.env_remove(name.as_str().unwrap_or_default());
        }
        for (name, value) in environment["set"].as_object().into_iter().flatten() {
            command.env(name, value.as_str().unwrap_or_default());
        }
        let mut child = command.spawn().expect("run the device's CLI");
        let mut stdin = child.stdin.take().expect("CLI stdin");
        if let Some(input) = input {
            stdin
                .write_all(input.to_string().as_bytes())
                .expect("write CLI input");
        }
        drop(stdin);
        let output = child.wait_with_output().expect("wait for the device's CLI");
        (
            output.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }

    fn owner_json(&self, device_id: &str, args: &[&str], input: Option<&Value>) -> Value {
        let (ok, output) = self.owner_cli(device_id, args, input);
        assert!(ok, "openbitfun {args:?} on {device_id} failed: {output}");
        serde_json::from_str(output.trim()).unwrap_or_else(|_| panic!("not JSON: {output}"))
    }
}

/// A1 delegates to the assistants of account B it is paired with: a task
/// whose command its owner approves on B1, a second task over B1's one-task
/// limit, another task, and a task on B2 that ends when B2's owner revokes
/// the connection. B's owners see each task in their audit log.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run by scripts/dev/sim-devices.mjs --check"]
async fn desktop_requester_delegates_to_other_accounts_over_connections() {
    use openbitfun_core::service::remote_connect::connection_rpc::{
        ConnectionContext, ConnectionRpc,
    };
    use openbitfun_core::service::remote_connect::connections::ConnectionStore;
    use openbitfun_core::service::remote_connect::RelayClient;

    let manifest = Manifest::read();
    let approval_connection = manifest.text("/connections/asksApproval/connectionId");
    let holds_connection = manifest.text("/connections/holds/connectionId");
    let b1 = manifest.text("/connections/asksApproval/deviceId");
    let b2 = manifest.text("/connections/holds/deviceId");
    let account_b = manifest.text("/accountB/userId");

    // A1's own device socket carries its connection calls.
    let (socket, mut relay_events) = RelayClient::new();
    socket
        .connect(&manifest.relay_url)
        .await
        .expect("A1 relay endpoint");
    socket
        .connect_authenticated(&manifest.session.token, "A1 Desktop")
        .await
        .expect("A1 device socket");
    tokio::spawn(async move { while relay_events.recv().await.is_some() {} });
    let capabilities = AccountClient::new()
        .relay_capabilities(&manifest.relay_url)
        .await
        .expect("relay capabilities");
    let connections = ConnectionRpc::new(
        Arc::new(socket),
        ConnectionContext {
            session: manifest.session.clone(),
            relay_url: manifest.relay_url.clone(),
            local_device_id: manifest.requester_device_id.clone(),
        },
        ConnectionStore::at(manifest.text("/requesterConnections")),
        capabilities,
    );
    // The connections A1 can use now, and what one of them answers.
    {
        use openbitfun_core::service::dispatch::ConnectionDispatchRpc;
        let peers = connections
            .usable_connections()
            .await
            .expect("A1 reads its connections");
        assert_eq!(peers.len(), 2, "{peers:?}");
        let probe = connections
            .invoke_connection(
                &approval_connection,
                "dispatch_target_probe",
                serde_json::json!({}),
            )
            .await
            .expect("probe B1 over its connection");
        assert!(
            probe.to_string().contains("cross_account_delegation_v1"),
            "{probe}"
        );
    }
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
    )
    .with_connections(Arc::new(connections));
    let port: &dyn RemoteAgentDelegationPort = &directory;
    let approval_address = manifest.address(&openbitfun_runtime_ports::remote_connection_target(
        &approval_connection,
    ));
    let holds_address = manifest.address(&openbitfun_runtime_ports::remote_connection_target(
        &holds_connection,
    ));

    // Condition 1: what account B granted to this connection is listed as
    // agents of that account; nothing else of B's is.
    let listed = port.list_agents(RemoteAgentListing::Refreshed).await;
    let mut other_account: Vec<String> = listed
        .iter()
        .filter(|agent| agent.account.is_some())
        .map(|agent| {
            assert_eq!(
                agent.account.as_ref().unwrap().user_id,
                account_b,
                "{agent:?}"
            );
            assert!(agent.availability.is_available(), "{agent:?}");
            agent.address.agent_type()
        })
        .collect();
    other_account.sort();
    let mut expected = vec![approval_address.agent_type(), holds_address.agent_type()];
    expected.sort();
    assert_eq!(other_account, expected, "{listed:?}");

    // Conditions 2 and 4: a task whose command B1's owner approves.
    let job = port
        .submit(submit_request(
            approval_address.clone(),
            "Create the marker file",
        ))
        .await
        .expect("submit approval job");
    let (waiting, _) = observe_until(port, &job, |page| !page.pending_permissions.is_empty()).await;
    let prompt = &waiting.pending_permissions[0];
    // A1 learns that the task waits, never what B1's owner is asked.
    assert!(prompt.resources.is_empty(), "{prompt:?}");
    assert!(
        !format!("{prompt:?}").contains("delegated.txt"),
        "{prompt:?}"
    );
    let refused = port
        .answer(&job, &prompt.request_id, PermissionReply::Once)
        .await
        .expect_err("A1 cannot approve on B1");
    assert!(
        refused
            .message
            .contains("only from the owner of the target device"),
        "{refused:?}"
    );
    // Condition 6: B1 runs one task at a time for this connection; another
    // is accepted and waits, queued, saying why.
    let queued = port
        .submit(submit_request(approval_address.clone(), "One more"))
        .await
        .expect("a task over the owner's limit is accepted");
    let (waiting, _) = observe_until(port, &queued, |page| page.hold.is_some()).await;
    assert_eq!(waiting.state, RemoteAgentJobState::Queued);
    assert_eq!(
        waiting.hold,
        Some(openbitfun_runtime_ports::RemoteAgentJobHold {
            reason: openbitfun_runtime_ports::RemoteAgentHoldReason::ConnectionLimit,
            limit: Some(1),
        })
    );
    // B1's owner sees the full request on B1 and approves it there.
    let status = manifest.owner_json(
        &b1,
        &["dispatch", "status"],
        Some(&serde_json::json!({ "jobId": job.job_id })),
    );
    let local = &status["pendingPermissions"][0];
    assert!(local.to_string().contains("delegated.txt"), "{status}");
    manifest.owner_json(
        &b1,
        &["dispatch", "answer"],
        Some(&serde_json::json!({
            "jobId": job.job_id,
            "requestId": local["requestId"],
            "reply": "once",
        })),
    );
    let (finished, _) = observe_until(port, &job, |page| page.state.is_terminal()).await;
    assert_eq!(
        finished.state,
        RemoteAgentJobState::Succeeded,
        "{:?}",
        finished.last_error
    );
    // The queued task starts on its own once the first ends.
    let (started, _) = observe_until(port, &queued, |page| page.state.is_terminal()).await;
    assert_eq!(
        started.state,
        RemoteAgentJobState::Succeeded,
        "{:?}",
        started.last_error
    );
    assert_eq!(started.hold, None);

    // Another task, now that B1 is free again.
    let message = port
        .submit(submit_request(
            approval_address.clone(),
            "Say hello across accounts",
        ))
        .await
        .expect("submit message job");
    let (answered, events) = observe_until(port, &message, |page| page.state.is_terminal()).await;
    assert_eq!(
        answered.state,
        RemoteAgentJobState::Succeeded,
        "{:?}",
        answered.last_error
    );
    assert!(!events.is_empty());

    // Condition 13: a later conversation resumes the first task. B1 continues
    // the same job, and with it the same session, from where it ended.
    let (before, _) = observe_until(port, &job, |page| page.state.is_terminal()).await;
    port.continue_job(&job, "resumed-turn", "Go on from where you stopped".to_string())
        .await
        .expect("resume the earlier task");
    let (resumed, _) = observe_until(port, &job, |page| {
        page.state.is_terminal() && page.cursor > before.cursor
    })
    .await;
    assert_eq!(
        resumed.state,
        RemoteAgentJobState::Succeeded,
        "{:?}",
        resumed.last_error
    );
    // A task B1 no longer keeps cannot be resumed.
    let gone = openbitfun_runtime_ports::RemoteAgentJob {
        job_id: format!("delegation-{}", uuid::Uuid::new_v4()),
        ..job.clone()
    };
    port.continue_job(&gone, "resumed-turn", "Go on".to_string())
        .await
        .expect_err("a task the device deleted is not resumed");

    // Condition 4: B2's owner revokes the connection while a task runs. The
    // task stops on B2, A1 reads that access was revoked, and new tasks are
    // refused.
    let held = port
        .submit(submit_request(holds_address.clone(), "Work until stopped"))
        .await
        .expect("submit long job");
    observe_until(port, &held, |page| {
        mentions(&page.events, STREAM_START_MARKER)
    })
    .await;
    manifest.owner_cli(&b2, &["connections", "revoke", &holds_connection], None);
    let deadline = Instant::now() + JOB_TIMEOUT;
    let ended = loop {
        let page = port.observe(&held, 0).await;
        if let Ok(page) = page {
            if page.state.is_terminal() {
                break page;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the revoked task did not end on A1"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    assert_eq!(ended.state, RemoteAgentJobState::Failed);
    assert!(
        ended
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("revoked")),
        "{ended:?}"
    );
    let deadline = Instant::now() + JOB_TIMEOUT;
    loop {
        let on_b2 = manifest.owner_json(
            &b2,
            &["dispatch", "status"],
            Some(&serde_json::json!({ "jobId": held.job_id })),
        );
        if on_b2["state"] == "failed" {
            assert!(
                on_b2["lastError"].to_string().contains("revoked"),
                "{on_b2}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the task kept running on B2: {on_b2}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    port.submit(submit_request(holds_address.clone(), "After revoking"))
        .await
        .expect_err("a revoked connection takes no new tasks");
    port.continue_job(&held, "resumed-turn", "Go on".to_string())
        .await
        .expect_err("a revoked connection resumes no earlier task");

    // Condition 8: each owner sees the tasks other accounts sent them.
    let audit = |device: &str| -> Vec<(String, String, String)> {
        manifest
            .owner_json(device, &["delegation", "audit", "--json"], None)
            .as_array()
            .unwrap()
            .iter()
            .map(|record| {
                let entry = &record["entry"];
                assert_eq!(
                    entry["peerUserId"],
                    manifest.session.user_id.as_str(),
                    "{record}"
                );
                (
                    entry["jobId"].as_str().unwrap().to_string(),
                    entry["event"].as_str().unwrap().to_string(),
                    entry["state"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect()
    };
    assert_eq!(
        audit(&b1),
        vec![
            (job.job_id.clone(), "submitted".into(), "".into()),
            (queued.job_id.clone(), "submitted".into(), "".into()),
            (job.job_id.clone(), "finished".into(), "succeeded".into()),
            (queued.job_id.clone(), "finished".into(), "succeeded".into()),
            (message.job_id.clone(), "submitted".into(), "".into()),
            (
                message.job_id.clone(),
                "finished".into(),
                "succeeded".into()
            ),
            (job.job_id.clone(), "finished".into(), "succeeded".into()),
        ]
    );
    assert_eq!(
        audit(&b2),
        vec![
            (held.job_id.clone(), "submitted".into(), "".into()),
            (held.job_id.clone(), "finished".into(), "failed".into()),
        ]
    );
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

    // Approval: answered from this device, carried out on the target. The
    // task first tries to rewrite its assistant; an approved write still
    // never lands (the CLI simulation checks the file).
    let job = port
        .submit(submit_request(
            manifest.address(&manifest.asks_approval),
            "Create the marker file",
        ))
        .await
        .expect("submit approval job");
    let waiting = loop {
        let (waiting, _) =
            observe_until(port, &job, |page| !page.pending_permissions.is_empty()).await;
        if waiting.pending_permissions[0].action == "bash" {
            break waiting;
        }
        port.answer(
            &job,
            &waiting.pending_permissions[0].request_id,
            PermissionReply::Once,
        )
        .await
        .expect("approve the write");
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
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
