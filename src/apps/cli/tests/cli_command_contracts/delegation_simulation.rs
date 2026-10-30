//! Simulated multi-account, multi-device agent delegation on one machine.
//!
//! A Relay runs in-process on 127.0.0.1 with an in-memory database. Accounts,
//! devices, and tokens are written straight into it, so no GitHub sign-in is
//! involved. Each target device is a real `openbitfun daemon run` process with
//! its own isolated storage and a mock model; nothing is installed as a
//! service. The requester here is this test, speaking the delegated-job wire
//! over the real encrypted account RPC; the desktop requester drives the same
//! simulation through `serve_simulated_devices` (see `scripts/dev/sim-devices.mjs`).
//!
//! This does not prove GitHub sign-in, real networks, mixed versions, or the
//! mobile and IM surfaces.

#[path = "../support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use openbitfun_core::service::remote_connect::{session_store, AccountClient, AccountSession};
use openbitfun_relay_service::{build_relay_router, db, MemoryAssetStore};
use openbitfun_services_core::dispatch_contract::DISPATCH_PROTOCOL_VERSION;
use openbitfun_services_integrations::remote_connect::device_crypto;
use serde_json::{json, Value};
use support::{CliTestEnvironment, MockOpenAiServer, STREAM_START_MARKER};

const ACCOUNT_A: &str = "1001";
const ACCOUNT_B: &str = "2002";
// Device ids are 32 hex digits, like the ones a real device derives.
const A1: &str = "00000000000000000000000000000a01";
const A2: &str = "00000000000000000000000000000a02";
const A3: &str = "00000000000000000000000000000a03";
const A4: &str = "00000000000000000000000000000a04";
const B1: &str = "00000000000000000000000000000b01";
const OPEN_AGENT_ID: &str = "helper";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const JOB_TIMEOUT: Duration = Duration::from_secs(90);

/// A Relay on a loopback port with a temporary account database.
struct LocalRelay {
    url: String,
    database: Arc<db::DbPool>,
    server: tokio::task::JoinHandle<()>,
    // A file, not `:memory:`: every pooled connection would open its own
    // empty in-memory database, and the pool replaces connections over time.
    _database_dir: tempfile::TempDir,
}

impl LocalRelay {
    /// Binds `host` (`0.0.0.0` lets another machine join); devices on this
    /// machine always reach it through loopback.
    async fn start(host: &str, port: u16) -> Self {
        let listener = tokio::net::TcpListener::bind((host, port))
            .await
            .expect("bind local relay");
        let port = listener.local_addr().expect("relay address").port();
        let url = format!("http://127.0.0.1:{port}");
        let database_dir = tempfile::tempdir().expect("relay database directory");
        let database_path = database_dir.path().join("relay.db");
        let database = Arc::new(
            db::connect(database_path.to_str().expect("relay database path"))
                .await
                .expect("relay database"),
        );
        let app = build_relay_router(
            Arc::new(MemoryAssetStore::new()),
            Instant::now(),
            database.clone(),
            env!("CARGO_PKG_VERSION"),
        );
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .expect("serve local relay");
        });
        Self {
            url,
            database,
            server,
            _database_dir: database_dir,
        }
    }

    /// Registers a device of `user_id` and returns its signed-in session.
    async fn seed_device(
        &self,
        user_id: &str,
        login: &str,
        device_id: &str,
        device_name: &str,
        kind: &str,
    ) -> AccountSession {
        db::UserRow::upsert_verified(&self.database, user_id, login)
            .await
            .expect("seed account");
        let secret = random_secret();
        db::DeviceRow::upsert(
            &self.database,
            device_id,
            user_id,
            device_name,
            Some(kind),
            Some(&device_crypto::public_key_base64(&secret)),
        )
        .await
        .expect("seed device");
        let token = db::AuthToken::create(&self.database, user_id, device_id)
            .await
            .expect("seed token");
        AccountSession::new(token.token, user_id.to_string(), secret)
    }
}

impl Drop for LocalRelay {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn random_secret() -> [u8; 32] {
    let mut secret = [0_u8; 32];
    secret[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    secret[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    secret
}

/// Writes a session a host restores at startup. The session store directory
/// is process-global, so sessions are written one after another.
fn write_session(home: &Path, relay_url: &str, session: &AccountSession, device_id: &str) {
    session_store::set_session_store_directory_for_test(home.to_path_buf());
    session_store::save_session_with_device(
        &session.token,
        &session.user_id,
        &session.master_key,
        relay_url,
        Some(device_id),
    )
    .expect("write simulated device session");
}

/// One simulated CLI device: isolated storage, a mock model, and a daemon
/// that holds the account routing connection.
struct SimulatedDevice {
    device_id: String,
    device_name: String,
    /// The CLI this device runs; the build output unless another machine
    /// names its copy.
    binary: PathBuf,
    environment: CliTestEnvironment,
    _model: MockOpenAiServer,
    daemon: Option<tokio::process::Child>,
}

impl SimulatedDevice {
    fn configure(
        device_id: &str,
        device_name: &str,
        model: MockOpenAiServer,
        tool_loop: bool,
    ) -> Self {
        let environment = CliTestEnvironment::new();
        if tool_loop {
            environment.configure_tool_loop_mock_model(model.base_url());
        } else {
            environment.configure_mock_model(model.base_url());
        }
        Self {
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
            binary: PathBuf::from(env!("CARGO_BIN_EXE_openbitfun")),
            environment,
            _model: model,
            daemon: None,
        }
    }

    /// Opens the Standard agent to `devices` for `task_kinds` through the
    /// shipped CLI command.
    fn open_agent(&self, devices: &[&str], task_kinds: &[&str]) {
        let mut command = self.environment.std_command_for(&self.binary);
        command.args(["delegation", "open", OPEN_AGENT_ID, "--agent", "Standard"]);
        for kind in task_kinds {
            command.args(["--task-kind", kind]);
        }
        for device in devices {
            command.args(["--device", device]);
        }
        let output = support::command_output_with_timeout(
            command.stdin(Stdio::null()),
            Duration::from_secs(60),
        );
        assert!(
            output.status.success(),
            "delegation open failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn start_daemon(&mut self) {
        let mut command =
            openbitfun_services_core::process_manager::create_tokio_command(&self.binary);
        self.environment.apply_tokio_environment(&mut command);
        command
            .args(["daemon", "run"])
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        // Kept beside the device's home, so a daemon that stops early can
        // be diagnosed; it is removed with the device.
        match std::fs::File::create(self.daemon_log_path()) {
            Ok(log) => {
                let stderr = log.try_clone().map(Stdio::from).unwrap_or(Stdio::null());
                command.stdout(Stdio::from(log)).stderr(stderr);
            }
            Err(_) => {
                command.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        self.daemon = Some(command.spawn().expect("start simulated device daemon"));
    }

    fn daemon_log_path(&self) -> PathBuf {
        self.environment
            .home_root()
            .parent()
            .unwrap_or_else(|| self.environment.home_root())
            .join("daemon.log")
    }
}

impl Drop for SimulatedDevice {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.start_kill();
        }
    }
}

/// Account A: requester A1 plus targets A2 (answers at once), A3 (holds its
/// model stream until cancelled), and A4 (runs a command that needs
/// approval). Account B: B1, which opens its agent to A1 as well.
struct Simulation {
    relay: LocalRelay,
    requester: AccountSession,
    devices: Vec<SimulatedDevice>,
}

impl Simulation {
    async fn start(relay_host: &str, relay_port: u16, requester_home: Option<&Path>) -> Self {
        let relay = LocalRelay::start(relay_host, relay_port).await;
        let requester = relay
            .seed_device(ACCOUNT_A, "sim-a", A1, "A1 Desktop", "desktop")
            .await;
        if let Some(home) = requester_home {
            write_session(home, &relay.url, &requester, A1);
        }

        let approving_model = MockOpenAiServer::tool_sequence(vec![(
            "ExecCommand".to_string(),
            json!({ "cmd": "touch delegated.txt" }),
        )]);
        let mut devices = vec![
            SimulatedDevice::configure(A2, "A2 Laptop", MockOpenAiServer::immediate(), false),
            SimulatedDevice::configure(A3, "A3 Server", MockOpenAiServer::gated(), false),
            SimulatedDevice::configure(A4, "A4 Workstation", approving_model, true),
            SimulatedDevice::configure(B1, "B1 Desktop", MockOpenAiServer::immediate(), false),
        ];
        for device in &mut devices {
            let (account, login) = if device.device_id.as_str() == B1 {
                (ACCOUNT_B, "sim-b")
            } else {
                (ACCOUNT_A, "sim-a")
            };
            let session = relay
                .seed_device(
                    account,
                    login,
                    &device.device_id,
                    &device.device_name,
                    "cli",
                )
                .await;
            write_session(
                device.environment.home_root(),
                &relay.url,
                &session,
                &device.device_id,
            );
            device.open_agent(&[A1], &["message"]);
            device.start_daemon();
        }

        let simulation = Self {
            relay,
            requester,
            devices,
        };
        simulation.wait_until_online().await;
        simulation
    }

    fn rpc(&self) -> RequesterRpc {
        RequesterRpc {
            relay_url: self.relay.url.clone(),
            session: self.requester.clone(),
        }
    }

    async fn wait_until_online(&self) {
        let expected = [A2, A3, A4];
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            let listed = AccountClient::new()
                .list_devices(&self.relay.url, &self.requester)
                .await
                .unwrap_or_default();
            if expected.iter().all(|id| {
                listed
                    .iter()
                    .any(|device| device.device_id == *id && device.online)
            }) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "simulated devices did not come online: {:?}",
                listed
                    .iter()
                    .map(|device| (&device.device_id, device.online))
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn device(&self, device_id: &str) -> &SimulatedDevice {
        self.devices
            .iter()
            .find(|device| device.device_id == device_id)
            .expect("simulated device")
    }

    /// What a requester host needs to join this simulation as A1.
    fn manifest(&self) -> Value {
        json!({
            "relayUrl": self.relay.url,
            "requester": {
                "deviceId": A1,
                "userId": self.requester.user_id,
                "token": self.requester.token,
                "deviceSecretHex": hex::encode(self.requester.master_key),
            },
            "openAgentId": OPEN_AGENT_ID,
            "devices": {
                "answers": A2,
                "holds": A3,
                "asksApproval": A4,
                "otherAccount": B1,
            },
            "deviceHomes": self.devices.iter().map(|device| {
                (device.device_id.clone(), json!(device.environment.home_root()))
            }).collect::<serde_json::Map<_, _>>(),
        })
    }
}

/// The delegated-job wire over the requester's account session.
struct RequesterRpc {
    relay_url: String,
    session: AccountSession,
}

impl RequesterRpc {
    async fn invoke(&self, device_id: &str, command: &str, args: Value) -> Result<Value, String> {
        let request = json!({ "cmd": "host_invoke", "command": command, "args": args });
        let raw = AccountClient::new()
            .device_rpc(
                &self.relay_url,
                &self.session,
                device_id,
                &request.to_string(),
            )
            .await
            .map_err(|error| format!("{error:#}"))?;
        let envelope: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(envelope.get("value").cloned().unwrap_or(Value::Null))
        } else {
            Err(envelope.to_string())
        }
    }

    async fn submit(&self, device_id: &str, prompt: &str) -> String {
        let job_id = format!("delegation-{}", uuid::Uuid::new_v4());
        let session_id = uuid::Uuid::new_v4().to_string();
        let ack = self
            .invoke(
                device_id,
                "dispatch_target_submit",
                json!({
                    "protocolVersion": DISPATCH_PROTOCOL_VERSION,
                    "jobId": job_id,
                    "sessionId": session_id,
                    "workspacePath": "",
                    "agentType": "Standard",
                    "prompt": prompt,
                    "approvalPolicy": "remote",
                    "title": "Simulated delegation",
                    "delegation": {
                        "openAgentId": OPEN_AGENT_ID,
                        "taskKind": "message",
                        "source": { "sessionId": "requester-session", "agentType": "Standard" },
                    },
                }),
            )
            .await
            .expect("submit delegated job");
        assert_eq!(ack["accepted"], true, "{ack}");
        assert_eq!(ack["jobId"], job_id.as_str(), "{ack}");
        job_id
    }

    /// Reads the job until `done` accepts a status page; returns that page
    /// and every event seen on the way.
    async fn observe_until(
        &self,
        device_id: &str,
        job_id: &str,
        mut done: impl FnMut(&Value) -> bool,
    ) -> (Value, Vec<Value>) {
        let deadline = Instant::now() + JOB_TIMEOUT;
        let mut cursor = 0;
        let mut events = Vec::new();
        loop {
            let page = self
                .invoke(
                    device_id,
                    "dispatch_target_status",
                    json!({ "jobId": job_id, "cursor": cursor }),
                )
                .await
                .expect("observe delegated job");
            cursor = page["cursor"].as_u64().unwrap_or(cursor);
            events.extend(page["events"].as_array().cloned().unwrap_or_default());
            if done(&page) {
                return (page, events);
            }
            let state = page["state"].as_str().unwrap_or_default();
            assert!(
                !matches!(state, "succeeded" | "failed" | "cancelled" | "interrupted"),
                "job ended as {state} before the expected state: {}; events: {}",
                page["lastError"],
                Value::Array(events)
            );
            assert!(
                Instant::now() < deadline,
                "job did not reach the expected state"
            );
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
}

fn terminal(page: &Value) -> bool {
    matches!(
        page["state"].as_str(),
        Some("succeeded" | "failed" | "cancelled" | "interrupted")
    )
}

fn mentions(events: &[Value], text: &str) -> bool {
    events.iter().any(|event| event.to_string().contains(text))
}

fn find_file(root: &Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.file_name().and_then(|value| value.to_str()) == Some(name) {
            return true;
        }
        path.is_dir() && find_file(&path, name)
    })
}

/// Runs `openbitfun connections ...` as `device` and returns its stdout.
fn connections_cli(device: &SimulatedDevice, args: &[&str]) -> (bool, String) {
    let mut command = device.environment.std_command_for(&device.binary);
    command
        .arg("connections")
        .args(args)
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost");
    let output =
        support::command_output_with_timeout(command.stdin(Stdio::null()), Duration::from_secs(60));
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

fn json_output(device: &SimulatedDevice, args: &[&str]) -> Value {
    let (ok, output) = connections_cli(device, args);
    assert!(ok, "connections {args:?} failed: {output}");
    serde_json::from_str(&output).unwrap_or_else(|_| panic!("not JSON: {output}"))
}

/// Two accounts pair one device each and call each other over the
/// connection. A1 is this test on its own device socket; B1 is a real daemon
/// managed through the shipped `openbitfun connections` command.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn devices_of_different_accounts_pair_and_call_over_a_connection() {
    use openbitfun_core::service::remote_connect::connection_rpc::{
        ConnectionContext, ConnectionRpc, CROSS_ACCOUNT_NOT_ENABLED,
    };
    use openbitfun_core::service::remote_connect::connections::{
        fingerprint_of, refusal_reason, same_fingerprint, ConnectionStore, ConnectionsClient,
    };
    use openbitfun_core::service::remote_connect::RelayClient;

    let relay = LocalRelay::start("127.0.0.1", 0).await;
    let requester = relay
        .seed_device(ACCOUNT_A, "sim-a", A1, "A1 Desktop", "desktop")
        .await;
    let mut target =
        SimulatedDevice::configure(B1, "B1 Desktop", MockOpenAiServer::immediate(), false);
    let target_session = relay
        .seed_device(ACCOUNT_B, "sim-b", B1, "B1 Desktop", "cli")
        .await;
    write_session(
        target.environment.home_root(),
        &relay.url,
        &target_session,
        B1,
    );
    target.start_daemon();

    // A1's own device socket: the Relay routes connection calls only from devices.
    let (socket, mut events) = RelayClient::new();
    socket.connect(&relay.url).await.expect("A1 relay endpoint");
    socket
        .connect_authenticated(&requester.token, "A1 Desktop")
        .await
        .expect("A1 device socket");
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    let socket = Arc::new(socket);

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while !AccountClient::new()
        .list_devices(&relay.url, &target_session)
        .await
        .unwrap_or_default()
        .iter()
        .any(|device| device.device_id == B1 && device.online)
    {
        assert!(
            Instant::now() < deadline,
            "B1 did not come online: {}",
            std::fs::read_to_string(target.daemon_log_path()).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let store_dir = tempfile::tempdir().expect("A1 connection store");
    let store = ConnectionStore::at(store_dir.path().join("connections.json"));
    let client = ConnectionsClient {
        relay_url: &relay.url,
        session: &requester,
        local_device_id: A1,
        store: &store,
    };

    // A1 invites; B1 sees who invited it, with the fingerprint from the link.
    let invite = client.create_invite(None).await.expect("create invite");
    let link = invite.link.to_string();
    let (ok, shown) = connections_cli(&target, &["accept", &link]);
    assert!(ok, "{shown}");
    assert!(shown.contains("A1 Desktop"), "{shown}");
    assert!(shown.contains(&client.local_fingerprint()), "{shown}");
    let accepted = json_output(&target, &["accept", &link, "--yes", "--json"]);
    let connection_id = accepted["connectionId"]
        .as_str()
        .expect("connection id")
        .to_owned();
    assert_eq!(accepted["status"], "pending");
    // The invite is single use.
    let (ok, again) = connections_cli(&target, &["accept", &link, "--yes"]);
    assert!(!ok && again.contains("invite_used"), "{again}");

    // A1 checks the fingerprint B1's owner reads out, then confirms.
    let target_view = json_output(&target, &["list", "--json"]);
    let target_fingerprint = target_view["localFingerprint"].as_str().unwrap().to_owned();
    let overview = client.overview(true).await.expect("A1 overview");
    let pending = &overview.connections[0];
    assert!(pending.awaiting_confirmation);
    assert!(same_fingerprint(
        pending.peer_fingerprint.as_deref().unwrap(),
        &target_fingerprint
    ));
    let wrong = fingerprint_of(
        &openbitfun_services_integrations::remote_connect::device_crypto::public_key_base64(
            &random_secret(),
        ),
    )
    .unwrap();
    let error = client.confirm(&connection_id, &wrong).await.unwrap_err();
    assert_eq!(refusal_reason(&error), Some("fingerprint_mismatch"));
    let confirmed = client
        .confirm(&connection_id, &target_fingerprint)
        .await
        .expect("confirm");
    assert_eq!(confirmed.status, "active");

    // Both sides list the other, active.
    let target_view = json_output(&target, &["list", "--json"]);
    let listed = &target_view["connections"][0];
    assert_eq!(listed["connectionId"], connection_id.as_str());
    assert_eq!(listed["status"], "active");
    assert_eq!(listed["usable"], true);
    assert_eq!(listed["peerDeviceId"], A1);

    // A call reaches B1's daemon, which opens it with the connection key and
    // refuses inside the encrypted answer: nothing is granted to connections.
    let capabilities = AccountClient::new()
        .relay_capabilities(&relay.url)
        .await
        .expect("relay capabilities");
    let rpc = ConnectionRpc::new(
        socket.clone(),
        ConnectionContext {
            session: requester.clone(),
            relay_url: relay.url.clone(),
            local_device_id: A1.into(),
        },
        store.clone(),
        capabilities,
    );
    let error = rpc
        .invoke_connection(&connection_id, "dispatch_target_catalog", json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), CROSS_ACCOUNT_NOT_ENABLED);

    // B1 revokes; both sides show it and new calls are refused by the Relay.
    let (ok, output) = connections_cli(&target, &["revoke", &connection_id]);
    assert!(ok, "{output}");
    let error = rpc
        .invoke_connection(&connection_id, "dispatch_target_catalog", json!({}))
        .await
        .unwrap_err();
    assert_eq!(
        refusal_reason(&error),
        Some("connection_revoked"),
        "{error}"
    );
    let overview = client.overview(true).await.expect("A1 overview");
    assert_eq!(overview.connections[0].status, "revoked");
    let target_view = json_output(&target, &["list", "--json"]);
    assert_eq!(target_view["connections"][0]["status"], "revoked");
    socket.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_account_devices_take_delegated_jobs_and_other_accounts_cannot_reach_them() {
    let simulation = Simulation::start("127.0.0.1", 0, None).await;
    let rpc = simulation.rpc();

    // Catalog: a same-account device lists the agent it opened to A1.
    let catalog = rpc
        .invoke(A2, "dispatch_target_catalog", json!({}))
        .await
        .expect("read the open-agent catalog");
    assert!(
        catalog["agents"]
            .as_array()
            .is_some_and(|agents| agents.iter().any(|agent| agent["id"] == OPEN_AGENT_ID)),
        "{catalog}"
    );
    // Another account's device is never routed to, even though it opened
    // its agent to A1's device id.
    let refused = rpc.invoke(B1, "dispatch_target_catalog", json!({})).await;
    assert!(
        refused.is_err(),
        "another account's device answered: {refused:?}"
    );

    // Message: the delegated job answers with the target's model output.
    let job = rpc.submit(A2, "Say hello from A2").await;
    let (finished, events) = rpc.observe_until(A2, &job, terminal).await;
    assert_eq!(finished["state"], "succeeded", "{finished}");
    assert!(mentions(&events, STREAM_START_MARKER), "{events:?}");

    // Cancel: a running job stops on its device. The target daemon is the
    // worker's parent here, unlike a one-shot SSH dispatch command.
    let job = rpc.submit(A3, "Work until cancelled").await;
    rpc.observe_until(A3, &job, |page| {
        mentions(
            page["events"].as_array().map(Vec::as_slice).unwrap_or(&[]),
            STREAM_START_MARKER,
        )
    })
    .await;
    let cancelled = rpc
        .invoke(A3, "dispatch_target_cancel", json!({ "jobId": job }))
        .await
        .expect("cancel delegated job");
    assert_eq!(cancelled["cancelled"], true, "{cancelled}");
    let (stopped, _) = rpc.observe_until(A3, &job, terminal).await;
    assert_eq!(stopped["state"], "cancelled", "{stopped}");

    // Approval: the target asks, the requester answers, the job finishes.
    let job = rpc.submit(A4, "Create the marker file").await;
    let (waiting, _) = rpc
        .observe_until(A4, &job, |page| {
            page["pendingPermissions"]
                .as_array()
                .is_some_and(|prompts| !prompts.is_empty())
        })
        .await;
    let prompt = &waiting["pendingPermissions"][0];
    assert_eq!(prompt["action"], "bash", "{prompt}");
    assert_eq!(
        prompt["resources"],
        json!(["touch delegated.txt"]),
        "{prompt}"
    );
    rpc.invoke(
        A4,
        "dispatch_target_answer",
        json!({
            "jobId": job,
            "requestId": prompt["requestId"],
            "reply": "once",
        }),
    )
    .await
    .expect("answer the target's prompt");
    let (approved, _) = rpc.observe_until(A4, &job, terminal).await;
    assert_eq!(approved["state"], "succeeded", "{approved}");
    assert!(
        find_file(
            simulation.device(A4).environment.home_root(),
            "delegated.txt"
        ),
        "the approved command ran on the target"
    );
}

/// Keeps the simulation running for `scripts/dev/sim-devices.mjs`. It writes
/// the manifest to `OPENBITFUN_SIM_MANIFEST` and stops once that file is
/// removed, or when stdin closes if `OPENBITFUN_SIM_STOP_ON_STDIN_EOF=1`.
/// `OPENBITFUN_SIM_REQUESTER_HOME` receives A1's session so a host started
/// there joins as A1; `OPENBITFUN_SIM_REQUESTER_USER_ROOT` receives its
/// config (see `write_requester_config`); `OPENBITFUN_SIM_RELAY_PORT` fixes
/// the port and `OPENBITFUN_SIM_RELAY_BIND=0.0.0.0` lets another machine
/// reach it. `OPENBITFUN_SIM_EXTERNAL_DEVICE_ID` seeds one more account-A
/// device for another machine; its session is in the manifest's `external`
/// entry for `serve_external_device`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "started by scripts/dev/sim-devices.mjs"]
async fn serve_simulated_devices() {
    let manifest_path = PathBuf::from(
        std::env::var_os("OPENBITFUN_SIM_MANIFEST").expect("OPENBITFUN_SIM_MANIFEST is required"),
    );
    let requester_home = std::env::var_os("OPENBITFUN_SIM_REQUESTER_HOME").map(PathBuf::from);
    let relay_port = std::env::var("OPENBITFUN_SIM_RELAY_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(0);
    if let Some(user_root) = std::env::var_os("OPENBITFUN_SIM_REQUESTER_USER_ROOT") {
        let models_from =
            std::env::var_os("OPENBITFUN_SIM_REQUESTER_MODELS_FROM").map(PathBuf::from);
        write_requester_config(Path::new(&user_root), models_from.as_deref());
    }
    let relay_host =
        std::env::var("OPENBITFUN_SIM_RELAY_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let simulation = Simulation::start(&relay_host, relay_port, requester_home.as_deref()).await;
    let mut manifest = simulation.manifest();
    if let Ok(device_id) = std::env::var("OPENBITFUN_SIM_EXTERNAL_DEVICE_ID") {
        let device_name = std::env::var("OPENBITFUN_SIM_EXTERNAL_DEVICE_NAME")
            .unwrap_or_else(|_| "External device".to_string());
        let session = simulation
            .relay
            .seed_device(ACCOUNT_A, "sim-a", &device_id, &device_name, "cli")
            .await;
        manifest["external"] = json!({
            "deviceId": device_id,
            "deviceName": device_name,
            "userId": session.user_id,
            "token": session.token,
            "deviceSecretHex": hex::encode(session.master_key),
            "requesterDeviceId": A1,
        });
    }

    let staged = manifest_path.with_extension("partial");
    std::fs::write(
        &staged,
        serde_json::to_vec_pretty(&manifest).expect("encode manifest"),
    )
    .expect("write simulation manifest");
    restrict_to_owner(&staged);
    std::fs::rename(&staged, &manifest_path).expect("publish simulation manifest");
    eprintln!("simulation ready: {}", manifest_path.display());

    wait_for_stop(&manifest_path).await;
    let _ = std::fs::remove_file(&manifest_path);
    eprintln!("simulation stopping");
}

/// Waits until `marker` is removed, or until stdin closes when
/// `OPENBITFUN_SIM_STOP_ON_STDIN_EOF=1`: a launcher that dies without
/// removing the marker still closes our stdin, so simulated daemons never
/// outlive it.
async fn wait_for_stop(marker: &Path) {
    let parent_gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if std::env::var("OPENBITFUN_SIM_STOP_ON_STDIN_EOF").as_deref() == Ok("1") {
        let parent_gone = parent_gone.clone();
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            parent_gone.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }
    while marker.exists() && !parent_gone.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[test]
fn targets_advertise_delegation_to_external_agents() {
    let environment = CliTestEnvironment::new();
    let mut command = environment.std_command();
    command.args(["dispatch", "probe"]).stdin(Stdio::null());
    let output = support::command_output_with_timeout(&mut command, Duration::from_secs(60));
    assert!(
        output.status.success(),
        "dispatch probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let probe: Value = serde_json::from_slice(&output.stdout).expect("probe JSON");
    let capabilities: Vec<&str> = probe["capabilities"]
        .as_array()
        .expect("capabilities")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        capabilities.contains(&"agent_delegation_v1"),
        "{capabilities:?}"
    );
    assert!(
        capabilities.contains(&"external_agent_executor_v1"),
        "{capabilities:?}"
    );
}

/// The edit the `coding` behavior makes: a `farewell` function next to the
/// requester's `greet`, a test for it, and a check that it imports.
const CODING_EDIT: &str = "printf '\\n\\ndef farewell(name: str) -> str:\\n    return f\"Goodbye, {name}!\"\\n' >> ibot/greet.py && printf '\\n\\ndef test_farewell():\\n    from ibot.greet import farewell\\n\\n    assert farewell(\"ibot\") == \"Goodbye, ibot!\"\\n' >> tests/test_greet.py && python3 -c 'from ibot.greet import farewell; print(farewell(\"ibot\"))'";

/// Runs one simulated device on another machine against a simulation
/// started with `OPENBITFUN_SIM_RELAY_BIND=0.0.0.0` and
/// `OPENBITFUN_SIM_EXTERNAL_DEVICE_ID`. `OPENBITFUN_SIM_EXTERNAL` names a
/// JSON file with the manifest's `external` entry plus `relayUrl` (the
/// simulation's address on the network) and `behavior` (`immediate`,
/// `gated`, `approval`, or `coding`); it is removed once read because it holds the
/// device secret. The session is written here, since session files are
/// bound to the machine that writes them. `OPENBITFUN_SIM_CLI_BIN` names
/// this machine's `openbitfun`. Stops when `<file>.running` is removed or
/// stdin closes (see `wait_for_stop`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run on another machine to join a simulation"]
async fn serve_external_device() {
    let request_path = PathBuf::from(
        std::env::var_os("OPENBITFUN_SIM_EXTERNAL").expect("OPENBITFUN_SIM_EXTERNAL is required"),
    );
    let request: Value = serde_json::from_slice(
        &std::fs::read(&request_path).expect("read external device request"),
    )
    .expect("decode external device request");
    let _ = std::fs::remove_file(&request_path);
    let text = |key: &str| {
        request[key]
            .as_str()
            .unwrap_or_else(|| panic!("external device request is missing {key}"))
            .to_string()
    };
    let mut secret = [0_u8; 32];
    hex::decode_to_slice(text("deviceSecretHex"), &mut secret).expect("device secret");
    let session = AccountSession::new(text("token"), text("userId"), secret);

    let (model, tool_loop) = match text("behavior").as_str() {
        "gated" => (MockOpenAiServer::gated(), false),
        "approval" => (
            MockOpenAiServer::tool_sequence(vec![(
                "ExecCommand".to_string(),
                json!({ "cmd": "touch delegated.txt" }),
            )]),
            true,
        ),
        // A coding task: one scripted edit in the delegated worktree, which
        // the target commits when the requester syncs the result.
        "coding" => (
            MockOpenAiServer::tool_sequence(vec![(
                "ExecCommand".to_string(),
                json!({ "cmd": CODING_EDIT }),
            )]),
            true,
        ),
        _ => (MockOpenAiServer::immediate(), false),
    };
    let task_kinds: &[&str] = if text("behavior") == "coding" {
        &["message", "coding"]
    } else {
        &["message"]
    };
    let mut device =
        SimulatedDevice::configure(&text("deviceId"), &text("deviceName"), model, tool_loop);
    if let Some(binary) = std::env::var_os("OPENBITFUN_SIM_CLI_BIN") {
        device.binary = PathBuf::from(binary);
    }
    write_session(
        device.environment.home_root(),
        &text("relayUrl"),
        &session,
        &device.device_id,
    );
    device.open_agent(&[&text("requesterDeviceId")], task_kinds);
    device.start_daemon();

    let marker = request_path.with_extension("running");
    std::fs::write(
        &marker,
        device.environment.home_root().display().to_string(),
    )
    .expect("write running marker");
    eprintln!("external device ready: {}", device.device_id);
    wait_for_stop(&marker).await;
    let _ = std::fs::remove_file(&marker);
    eprintln!("external device stopping");
}

/// Writes A1's default config. With `models_from`, the `ai` section is read
/// from that config (normally the developer's own) so A1 can run a real
/// model; nothing else is taken from it, so no bot, relay, or workspace
/// setting of the real profile reaches the simulation.
fn write_requester_config(user_root: &Path, models_from: Option<&Path>) {
    let mut config =
        serde_json::to_value(openbitfun_core::service::config::GlobalConfig::default())
            .expect("serialize default requester config");
    if let Some(source) = models_from {
        let source: Value = serde_json::from_slice(
            &std::fs::read(source).expect("read the config to take models from"),
        )
        .expect("decode the config to take models from");
        if let Some(ai) = source.get("ai") {
            config["ai"] = ai.clone();
        }
    }
    let config_dir = user_root.join("config");
    std::fs::create_dir_all(&config_dir).expect("create requester config directory");
    let path = config_dir.join("app.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&config).expect("encode requester config"),
    )
    .expect("write requester config");
    restrict_to_owner(&path);
}

/// The manifest carries A1's test session secret.
fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}
