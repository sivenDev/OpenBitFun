use super::*;
use openbitfun_services_integrations::remote_connect::connections::{
    derive_connection_key, local_public_key_base64, refusal_reason, ConnectionRecord,
    ConnectionRole, StoredConnections,
};
use std::sync::Mutex;

const RELAY: &str = "https://relay.example";
const CONNECTION: &str = "conn-1";

fn secret(byte: u8) -> [u8; 32] {
    let mut value = [byte; 32];
    value[31] = byte.wrapping_mul(3).wrapping_add(1);
    value
}

struct Device {
    context: ConnectionContext,
    store: ConnectionStore,
    _dir: tempfile::TempDir,
}

fn device(user: &str, device_id: &str, secret: [u8; 32]) -> Device {
    let dir = tempfile::tempdir().unwrap();
    Device {
        context: ConnectionContext {
            session: AccountSession::new(format!("token-{user}"), user.into(), secret),
            relay_url: RELAY.into(),
            local_device_id: device_id.into(),
        },
        store: ConnectionStore::at(dir.path().join("connections.json")),
        _dir: dir,
    }
}

fn pin(local: &Device, peer: &Device, role: ConnectionRole) {
    let record = ConnectionRecord {
        connection_id: CONNECTION.into(),
        relay_url: RELAY.into(),
        local_device_id: local.context.local_device_id.clone(),
        role,
        status: "active".into(),
        peer_user_id: peer.context.session.user_id.clone(),
        peer_display_name: format!("{} name", peer.context.session.user_id),
        peer_device_id: peer.context.local_device_id.clone(),
        peer_device_name: "Laptop".into(),
        pinned_public_key: Some(local_public_key_base64(&peer.context.session.master_key)),
        reported_public_key: None,
        created_at: 1,
        confirmed_at: Some(2),
        revoked_at: None,
        disabled_reason: None,
    };
    local
        .store
        .save(&StoredConnections {
            records: vec![record],
            unreadable: Vec::new(),
        })
        .unwrap();
}

/// Echoes the call, so a test sees what reached the handler.
struct Echo;

#[async_trait]
impl ConnectionRequestHandler for Echo {
    async fn handle(
        &self,
        caller: &ConnectionCaller,
        command: &str,
        args: Value,
    ) -> std::result::Result<Value, String> {
        Ok(json!({ "caller": caller.peer_device_id, "command": command, "args": args }))
    }
}

enum Behavior {
    Deliver,
    /// The relay rewrites the routed command it reports to the target.
    RewriteCommand(&'static str),
    /// The relay reports another source device.
    SpoofSource(&'static str),
    Refuse(&'static str),
    /// The relay delivers the same request a second time and returns the
    /// second answer.
    Replay,
    /// The relay answers every later call with the first answer it saw.
    SwapInFirstAnswer,
}

/// An in-process relay: delivers each call to `target` as the real relay
/// would, recording the ciphertext it saw.
struct FakeRelay {
    target: Device,
    source_account: String,
    source_device: String,
    handler: Box<dyn ConnectionRequestHandler>,
    behavior: Behavior,
    seen: Mutex<Vec<String>>,
    first_answer: Mutex<Option<Value>>,
}

#[async_trait]
impl ConnectionTransport for FakeRelay {
    async fn request_connection(
        &self,
        connection_id: &str,
        command: &str,
        encrypted_data: &str,
        nonce: &str,
    ) -> Result<Value> {
        self.seen.lock().unwrap().push(encrypted_data.to_owned());
        let (command, source) = match self.behavior {
            Behavior::Deliver | Behavior::Replay | Behavior::SwapInFirstAnswer => {
                (command, self.source_device.as_str())
            }
            Behavior::RewriteCommand(other) => (other, self.source_device.as_str()),
            Behavior::SpoofSource(other) => (command, other),
            Behavior::Refuse(reason) => return Err(route_refusal(reason, "refused by test relay")),
        };
        let request = IncomingConnectionRequest {
            connection_id: connection_id.into(),
            command: command.into(),
            source_account_id: self.source_account.clone(),
            source_device_id: source.into(),
            encrypted_data: encrypted_data.into(),
            nonce: nonce.into(),
        };
        let deliver = || {
            answer_connection_request(
                &self.target.context,
                &self.target.store,
                &request,
                self.handler.as_ref(),
            )
        };
        let answer = deliver().await;
        match self.behavior {
            Behavior::Replay => Ok(deliver().await),
            Behavior::SwapInFirstAnswer => {
                let mut first = self.first_answer.lock().unwrap();
                Ok(first.get_or_insert(answer).clone())
            }
            _ => Ok(answer),
        }
    }
}

fn paired(
    handler: Box<dyn ConnectionRequestHandler>,
    behavior: Behavior,
    capabilities: &[&str],
) -> (ConnectionRpc, Arc<FakeRelay>, Device) {
    // As the relay connection does before registering for connection calls.
    connections::replay_guard();
    let requester = device("user-a", "device-a", secret(1));
    let target = device("user-b", "device-b", secret(2));
    pin(&requester, &target, ConnectionRole::Inviter);
    pin(&target, &requester, ConnectionRole::Acceptor);
    let relay = Arc::new(FakeRelay {
        source_account: requester.context.session.user_id.clone(),
        source_device: requester.context.local_device_id.clone(),
        target,
        handler,
        behavior,
        seen: Mutex::new(Vec::new()),
        first_answer: Mutex::new(None),
    });
    let rpc = ConnectionRpc::new(
        relay.clone(),
        requester.context.clone(),
        requester.store.clone(),
        capabilities.iter().map(|value| value.to_string()).collect(),
    );
    (rpc, relay, requester)
}

const SUPPORTED: &[&str] = &[
    "account_transport_v1",
    RELAY_CAPABILITY_CROSS_ACCOUNT_ROUTING,
];

#[tokio::test]
async fn request_and_answer_round_trip_under_the_connection_key() {
    let (rpc, relay, _requester) = paired(Box::new(Echo), Behavior::Deliver, SUPPORTED);
    let answer = rpc
        .invoke_connection(
            CONNECTION,
            "dispatch_target_probe",
            json!({"secret": "plan"}),
        )
        .await
        .unwrap();
    assert_eq!(
        answer,
        json!({"caller": "device-a", "command": "dispatch_target_probe", "args": {"secret": "plan"}})
    );
    // The relay saw only ciphertext, and it opens with the connection key.
    let seen = relay.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].contains("plan"));
    let key = derive_connection_key(
        &secret(2),
        &openbitfun_services_integrations::remote_connect::device_crypto::public_key(&secret(1)),
        CONNECTION,
    )
    .unwrap();
    let to_target = Direction {
        connection_id: CONNECTION,
        from_device_id: "device-a",
        to_device_id: "device-b",
    };
    assert!(
        connections::open_request(&key, to_target, "dispatch_target_probe", &seen[0], "").is_err()
    );
}

#[tokio::test]
async fn a_replayed_request_is_refused_by_the_target() {
    let (rpc, _relay, _requester) = paired(Box::new(Echo), Behavior::Replay, SUPPORTED);
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_submit", json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "connection request was already received");
}

#[tokio::test]
async fn an_older_answer_swapped_in_by_the_relay_is_refused() {
    let (rpc, _relay, _requester) = paired(Box::new(Echo), Behavior::SwapInFirstAnswer, SUPPORTED);
    let first = rpc
        .invoke_connection(CONNECTION, "dispatch_target_status", json!({"n": 1}))
        .await
        .unwrap();
    assert_eq!(first["args"], json!({"n": 1}));
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_status", json!({"n": 2}))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "connection answer does not match its request"
    );
}

#[tokio::test]
async fn a_request_reflected_back_to_its_sender_is_refused() {
    // Both devices are pinned to each other; the relay hands device A's own
    // request back to device A, reporting device B as the source.
    let (rpc, relay, requester) = paired(Box::new(Echo), Behavior::Deliver, SUPPORTED);
    rpc.invoke_connection(CONNECTION, "dispatch_target_probe", json!({}))
        .await
        .unwrap();
    let record = requester
        .store
        .usable(CONNECTION, RELAY, "device-a")
        .unwrap();
    let key = record
        .connection_key(&requester.context.session.master_key)
        .unwrap();
    let sealed = connections::seal_request(
        &key,
        Direction {
            connection_id: CONNECTION,
            from_device_id: "device-a",
            to_device_id: "device-b",
        },
        "dispatch_target_probe",
        &json!({}),
    )
    .unwrap();
    let reflected = IncomingConnectionRequest {
        connection_id: CONNECTION.into(),
        command: "dispatch_target_probe".into(),
        source_account_id: relay.target.context.session.user_id.clone(),
        source_device_id: relay.target.context.local_device_id.clone(),
        encrypted_data: sealed.encrypted_data,
        nonce: sealed.nonce,
    };
    let answer =
        answer_connection_request(&requester.context, &requester.store, &reflected, &Echo).await;
    assert_eq!(answer, json!({"refused": "request_rejected"}));
}

#[tokio::test]
async fn a_device_that_granted_nothing_refuses_inside_the_encrypted_answer() {
    let (rpc, _relay, _requester) = paired(
        Box::new(RefuseConnectionRequests),
        Behavior::Deliver,
        SUPPORTED,
    );
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_catalog", json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), CROSS_ACCOUNT_NOT_ENABLED);
}

#[tokio::test]
async fn a_relay_without_cross_account_routing_is_refused_before_sending() {
    let (rpc, relay, _requester) =
        paired(Box::new(Echo), Behavior::Deliver, &["account_transport_v1"]);
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_probe", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refusal_reason(&error), Some("relay_unsupported"));
    assert!(relay.seen.lock().unwrap().is_empty());
    assert!(require_connection_support(&[]).is_err());
    assert!(require_connection_support(&[RELAY_CAPABILITY_CROSS_ACCOUNT_ROUTING.into()]).is_ok());
}

#[tokio::test]
async fn non_dispatch_commands_never_leave_the_device() {
    let (rpc, relay, _requester) = paired(Box::new(Echo), Behavior::Deliver, SUPPORTED);
    let error = rpc
        .invoke_connection(CONNECTION, "read_file", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refusal_reason(&error), Some("command_not_routable"));
    assert!(relay.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn revocation_by_the_relay_is_recorded_and_later_calls_stay_local() {
    let (rpc, relay, requester) = paired(
        Box::new(Echo),
        Behavior::Refuse("connection_revoked"),
        SUPPORTED,
    );
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_probe", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refusal_reason(&error), Some("connection_revoked"));
    let record = requester
        .store
        .load()
        .unwrap()
        .find(CONNECTION)
        .cloned()
        .unwrap();
    assert_eq!(record.status, "revoked");
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_probe", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refusal_reason(&error), Some("connection_revoked"));
    assert_eq!(
        relay.seen.lock().unwrap().len(),
        1,
        "the second call is not sent"
    );
}

#[tokio::test]
async fn a_key_change_reported_by_the_relay_disables_the_connection() {
    let (rpc, _relay, requester) = paired(
        Box::new(Echo),
        Behavior::Refuse("connection_key_changed"),
        SUPPORTED,
    );
    let _ = rpc
        .invoke_connection(CONNECTION, "dispatch_target_probe", json!({}))
        .await
        .unwrap_err();
    let record = requester
        .store
        .load()
        .unwrap()
        .find(CONNECTION)
        .cloned()
        .unwrap();
    assert_eq!(record.unusable_reason(), Some("key_changed"));
}

#[tokio::test]
async fn the_target_checks_routing_against_its_own_record() {
    let (rpc, _relay, _requester) = paired(
        Box::new(Echo),
        Behavior::RewriteCommand("dispatch_target_cancel"),
        SUPPORTED,
    );
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_probe", json!({}))
        .await
        .unwrap_err();
    // The target cannot open it under the reported routing, so it has no
    // authentic request id to answer under and refuses in plaintext.
    assert_eq!(refusal_reason(&error), Some("request_rejected"), "{error}");

    let (rpc, _relay, _requester) =
        paired(Box::new(Echo), Behavior::SpoofSource("device-z"), SUPPORTED);
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_probe", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refusal_reason(&error), Some("connection_mismatch"));
}

#[tokio::test]
async fn a_target_that_revoked_locally_refuses_without_a_key() {
    let (rpc, relay, _requester) = paired(Box::new(Echo), Behavior::Deliver, SUPPORTED);
    relay
        .target
        .store
        .note_refusal(CONNECTION, "connection_revoked")
        .unwrap();
    let error = rpc
        .invoke_connection(CONNECTION, "dispatch_target_probe", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refusal_reason(&error), Some("connection_revoked"));
}
