//! Channel contracts between two sharing identities on one machine.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use openbitfun_services_integrations::lan_share::{
    connect, ChannelConfig, LanConnection, LanError, LanListener, PeerIdentity, RequestHandler,
    ShareIdentity,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;

fn config(name: &str) -> ChannelConfig {
    let mut config = ChannelConfig::new(Arc::new(ShareIdentity::generate(name, name).unwrap()));
    config.allow_loopback = true;
    config.handshake_timeout = Duration::from_secs(2);
    config
}

/// Echoes requests and keeps every connection it was given.
#[derive(Default)]
struct Echo {
    connections: Mutex<Vec<LanConnection>>,
}

#[async_trait]
impl RequestHandler for Echo {
    async fn handle(
        &self,
        peer: &PeerIdentity,
        method: &str,
        body: Value,
    ) -> Result<Value, String> {
        match method {
            "echo" => Ok(json!({ "from": peer.device_id, "body": body })),
            "big" => Ok(json!("x".repeat(body.as_u64().unwrap_or(0) as usize))),
            _ => Err(format!("unknown method {method}")),
        }
    }

    async fn connected(&self, connection: LanConnection) {
        self.connections.lock().await.push(connection);
    }
}

async fn listen(config: ChannelConfig) -> (LanListener, Arc<Echo>, SocketAddr) {
    let echo = Arc::new(Echo::default());
    let listener = LanListener::bind(config, echo.clone(), &[Ipv4Addr::LOCALHOST])
        .await
        .unwrap();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, listener.port()));
    (listener, echo, address)
}

async fn server_side(echo: &Echo) -> LanConnection {
    for _ in 0..100 {
        if let Some(connection) = echo.connections.lock().await.first() {
            return connection.clone();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the listener never reported the connection");
}

#[tokio::test]
async fn both_sides_call_and_push_over_an_encrypted_channel() {
    let server = config("owner");
    let server_identity = server.identity.clone();
    let (_listener, echo, address) = listen(server).await;
    let client = config("user");
    let client_echo: Arc<dyn RequestHandler> = Arc::new(Echo::default());

    let connection = connect(
        &client,
        address,
        Some(&server_identity.public_key()),
        Some(client_echo),
    )
    .await
    .unwrap();

    assert_eq!(connection.peer().device_id, server_identity.device_id());
    assert_eq!(connection.peer().display_name, "owner");
    let answer = connection.call("echo", json!({ "n": 1 })).await.unwrap();
    assert_eq!(answer["from"], client.identity.device_id());
    assert_eq!(answer["body"], json!({ "n": 1 }));
    assert_eq!(
        connection.call("nope", Value::Null).await.unwrap_err(),
        LanError::Remote("unknown method nope".to_string())
    );

    let owner_side = server_side(&echo).await;
    assert_eq!(owner_side.peer().device_id, client.identity.device_id());
    let mut pushes = connection.take_pushes().unwrap();
    owner_side
        .push("granted", json!({ "assistant": "x" }))
        .await
        .unwrap();
    let push = tokio::time::timeout(Duration::from_secs(2), pushes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(push.topic, "granted");
    assert_eq!(push.body, json!({ "assistant": "x" }));
    let back = owner_side.call("echo", json!("hi")).await.unwrap();
    assert_eq!(back["from"], server_identity.device_id());

    connection.close().await;
    tokio::time::timeout(Duration::from_secs(2), owner_side.closed())
        .await
        .unwrap();
    assert_eq!(
        owner_side.call("echo", Value::Null).await.unwrap_err(),
        LanError::Closed
    );
}

#[tokio::test]
async fn a_key_that_does_not_match_the_advertisement_is_refused() {
    let (_listener, _echo, address) = listen(config("owner")).await;
    // A different key, even one whose short code matched, is refused.
    let impostor = ShareIdentity::generate("impostor", "impostor").unwrap();
    let error = connect(&config("user"), address, Some(&impostor.public_key()), None)
        .await
        .unwrap_err();
    assert_eq!(error, LanError::PeerKeyMismatch);
}

#[tokio::test]
async fn missing_capabilities_end_the_handshake_as_needing_an_update() {
    let mut server = config("owner");
    server.capabilities = vec!["assistant_share_v1".to_string()];
    server.required_capabilities = vec!["assistant_share_v1".to_string()];
    let (_listener, _echo, address) = listen(server).await;

    let error = connect(&config("old"), address, None, None)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        LanError::NeedsUpdate {
            missing: vec!["assistant_share_v1".to_string()]
        }
    );

    let mut demanding = config("new");
    demanding.capabilities = vec!["assistant_share_v1".to_string()];
    demanding.required_capabilities = vec!["assistant_share_v2".to_string()];
    let error = connect(&demanding, address, None, None).await.unwrap_err();
    assert_eq!(
        error,
        LanError::PeerNeedsUpdate {
            missing: vec!["assistant_share_v2".to_string()]
        }
    );
}

#[tokio::test]
async fn a_silent_peer_times_out() {
    let silent = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = silent.local_addr().unwrap();
    let _held = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = silent.accept().await {
            held.push(stream);
        }
    });
    let mut client = config("user");
    client.handshake_timeout = Duration::from_millis(300);
    assert_eq!(
        connect(&client, address, None, None).await.unwrap_err(),
        LanError::Timeout
    );
}

#[tokio::test]
async fn a_listener_drops_peers_that_never_finish_the_handshake() {
    let mut server = config("owner");
    server.handshake_timeout = Duration::from_millis(200);
    server.max_pending_handshakes = 1;
    let (_listener, _echo, address) = listen(server).await;

    let stalled = tokio::net::TcpStream::connect(address).await.unwrap();
    // The single handshake place is taken, so this peer is turned away.
    let mut client = config("user");
    client.handshake_timeout = Duration::from_millis(500);
    assert!(connect(&client, address, None, None).await.is_err());

    // Once the stalled peer has timed out, the place is free again.
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(stalled);
    assert!(connect(&config("user"), address, None, None).await.is_ok());
}

#[tokio::test]
async fn oversized_messages_are_refused_in_both_directions() {
    let mut server = config("owner");
    server.max_frame_bytes = 1024;
    let (_listener, _echo, address) = listen(server).await;
    let mut client = config("user");
    client.max_frame_bytes = 1024;
    let connection = connect(&client, address, None, None).await.unwrap();

    assert_eq!(
        connection
            .call("echo", json!("y".repeat(2048)))
            .await
            .unwrap_err(),
        LanError::TooLarge(1024)
    );
    let error = connection.call("big", json!(4096)).await.unwrap_err();
    assert!(matches!(error, LanError::Remote(message) if message.contains("limit")));
    assert!(connection.call("echo", json!("ok")).await.is_ok());
}

#[tokio::test]
async fn non_private_addresses_are_refused() {
    let (_listener, _echo, address) = listen(config("owner")).await;
    let mut strict = config("user");
    strict.allow_loopback = false;
    assert!(matches!(
        connect(&strict, address, None, None).await.unwrap_err(),
        LanError::Refused(_)
    ));

    let mut strict_server = config("owner");
    strict_server.allow_loopback = false;
    assert!(LanListener::bind(
        strict_server,
        Arc::new(Echo::default()),
        &[Ipv4Addr::LOCALHOST]
    )
    .await
    .is_err());
}

#[tokio::test]
async fn a_listener_takes_its_preferred_port_and_any_free_one_when_it_is_taken() {
    // A port the system just handed out is free now, as far as anyone can tell.
    let free = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut first = config("owner");
    first.listen_port = free;
    let (preferred, _, _) = listen(first).await;
    assert_eq!(preferred.port(), free);

    let mut second = config("other");
    second.listen_port = free;
    let (fallback, _, _) = listen(second).await;
    assert_ne!(fallback.port(), free);
    assert_ne!(fallback.port(), 0);
}

#[tokio::test]
async fn an_unknown_device_is_reached_by_address_alone() {
    let server = config("owner");
    let server_identity = server.identity.clone();
    let (_listener, _echo, address) = listen(server).await;

    let connection = connect(&config("user"), address, None, None).await.unwrap();
    let peer = connection.peer();
    assert_eq!(peer.public_key, server_identity.public_key());
    assert_eq!(peer.device_id, server_identity.device_id());
    assert_eq!(peer.fingerprint, server_identity.fingerprint());
    assert_eq!(peer.address, address);
}
