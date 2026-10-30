use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn secret(byte: u8) -> [u8; 32] {
    let mut value = [byte; 32];
    value[0] = byte.wrapping_add(1);
    value
}

fn public_b64(secret: &[u8; 32]) -> String {
    local_public_key_base64(secret)
}

fn relay_connection(id: &str, local_device: &str, peer_key: &str, status: &str) -> RelayConnection {
    RelayConnection {
        connection_id: id.into(),
        status: status.into(),
        role: ConnectionRole::Inviter,
        local_device_id: local_device.into(),
        peer: RelayPeer {
            user_id: "user-b".into(),
            display_name: "Bea".into(),
            device_id: "device-b".into(),
            device_name: "Bea's laptop".into(),
            public_key: peer_key.into(),
        },
        created_at: 10,
        confirmed_at: None,
        revoked_at: None,
        revoked_reason: None,
    }
}

#[test]
fn fingerprint_is_grouped_and_compares_loosely() {
    let value = fingerprint(&device_crypto::public_key(&secret(1)));
    let groups: Vec<_> = value.split(' ').collect();
    assert_eq!(groups.len(), 8);
    assert!(groups.iter().all(|group| group.len() == 4));
    assert!(same_fingerprint(
        &value,
        &value.replace(' ', "").to_lowercase()
    ));
    assert!(!same_fingerprint(
        &value,
        &fingerprint(&device_crypto::public_key(&secret(2)))
    ));
    assert!(!same_fingerprint("", ""));
}

#[test]
fn connection_key_is_shared_and_bound_to_the_connection() {
    let (a, b) = (secret(3), secret(4));
    let (a_pub, b_pub) = (device_crypto::public_key(&a), device_crypto::public_key(&b));
    let on_a = derive_connection_key(&a, &b_pub, "conn-1").unwrap();
    let on_b = derive_connection_key(&b, &a_pub, "conn-1").unwrap();
    assert_eq!(on_a, on_b);
    assert_ne!(on_a, derive_connection_key(&a, &b_pub, "conn-2").unwrap());
    // Same-account pair keys use another salt, so they never coincide.
    assert_ne!(on_a, device_crypto::derive_message_key(&a, &b_pub).unwrap());
    assert!(derive_connection_key(&a, &[0u8; 32], "conn-1").is_err());
}

#[test]
fn invite_link_round_trips_and_rejects_incomplete_links() {
    let link = InviteLink {
        relay_url: "https://relay.example.com/base".into(),
        code: "ABCD-EFGH&x".into(),
        fingerprint: "0123ABCD".into(),
    };
    assert_eq!(InviteLink::parse(&link.to_string()).unwrap(), link);
    assert!(InviteLink::parse("https://example.com/?code=1").is_err());
    assert!(InviteLink::parse("openbitfun://connect?relay=https%3A%2F%2Fr.example&fp=AB").is_err());
    assert!(InviteLink::parse("openbitfun://connect?relay=ftp%3A%2F%2Fr&code=A&fp=AB").is_err());
}

const A_TO_B: Direction<'static> = Direction {
    connection_id: "conn-1",
    from_device_id: "device-a",
    to_device_id: "device-b",
};
const B_TO_A: Direction<'static> = Direction {
    connection_id: "conn-1",
    from_device_id: "device-b",
    to_device_id: "device-a",
};

#[test]
fn requests_open_only_with_matching_routing_and_direction() {
    let key = [7u8; 32];
    let sealed = seal_request(&key, A_TO_B, "dispatch_target_submit", &json!({"x": 1})).unwrap();
    let open = |direction: Direction<'_>, command: &str, key: &[u8; 32]| {
        open_request(
            key,
            direction,
            command,
            &sealed.encrypted_data,
            &sealed.nonce,
        )
    };
    let opened = open(A_TO_B, "dispatch_target_submit", &key).unwrap();
    assert_eq!(opened.args, json!({"x": 1}));
    assert_eq!(opened.request_id, sealed.request_id);
    // A relay that rewrites the routed command or connection is caught.
    assert!(open(A_TO_B, "dispatch_target_cancel", &key).is_err());
    let other_connection = Direction {
        connection_id: "conn-2",
        ..A_TO_B
    };
    assert!(open(other_connection, "dispatch_target_submit", &key).is_err());
    assert!(open(A_TO_B, "dispatch_target_submit", &[8u8; 32]).is_err());
    // Reflection: A's own request handed back to A as if B sent it.
    assert!(open(B_TO_A, "dispatch_target_submit", &key).is_err());
}

#[test]
fn answers_must_echo_their_request_and_direction() {
    let key = [7u8; 32];
    let (data, nonce) = seal_response(&key, B_TO_A, "req-1", Ok(json!({"ok": "yes"}))).unwrap();
    assert_eq!(
        open_response(&key, B_TO_A, "req-1", &data, &nonce).unwrap(),
        json!({"ok": "yes"})
    );
    // An older answer swapped in for another request is refused.
    assert!(open_response(&key, B_TO_A, "req-2", &data, &nonce).is_err());
    // An answer reflected back to the device that sent it is refused.
    assert!(open_response(&key, A_TO_B, "req-1", &data, &nonce).is_err());
    let (data, nonce) = seal_response(&key, B_TO_A, "req-1", Err("refused".into())).unwrap();
    assert_eq!(
        open_response(&key, B_TO_A, "req-1", &data, &nonce)
            .unwrap_err()
            .to_string(),
        "refused"
    );
}

#[test]
fn replay_guard_refuses_repeated_and_stale_requests() {
    let key = [7u8; 32];
    let sealed = seal_request(&key, A_TO_B, "dispatch_target_submit", &json!({})).unwrap();
    let opened = open_request(
        &key,
        A_TO_B,
        "dispatch_target_submit",
        &sealed.encrypted_data,
        &sealed.nonce,
    )
    .unwrap();
    let now = opened.issued_at;
    let guard = ReplayGuard::started_at(now);
    guard.admit("conn-1", &opened, now).unwrap();
    assert!(
        guard.admit("conn-1", &opened, now + 1).is_err(),
        "a replay is refused"
    );
    // The same id on another connection is a different request.
    assert!(guard.admit("conn-2", &opened, now + 1).is_ok());
    let fresh = OpenedRequest {
        request_id: "other".into(),
        ..opened.clone()
    };
    assert!(guard
        .admit("conn-1", &fresh, now + REQUEST_MAX_AGE_SECONDS + 1)
        .is_err());
    assert!(guard.admit("conn-1", &fresh, now).is_ok());

    // After a restart the new guard knows no ids, so anything issued before
    // it started is refused, even inside the age window.
    let restarted = ReplayGuard::started_at(now + 10);
    assert!(restarted.admit("conn-1", &opened, now + 11).is_err());
    let later = OpenedRequest {
        request_id: "later".into(),
        issued_at: now + 10,
        ..opened
    };
    assert!(restarted.admit("conn-1", &later, now + 11).is_ok());
}

#[test]
fn store_keeps_unreadable_entries_and_disables_them() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.json"));
    let good = ConnectionRecord {
        pinned_public_key: Some(public_b64(&secret(5))),
        status: "active".into(),
        ..ConnectionRecord::from_relay(
            "https://relay.example",
            &relay_connection("conn-good", "device-a", &public_b64(&secret(5)), "active"),
        )
    };
    let raw = json!({
        "version": 1,
        "connections": [
            serde_json::to_value(&good).unwrap(),
            {"connectionId": "conn-bad", "role": 42},
        ],
    });
    std::fs::write(store.path(), raw.to_string()).unwrap();

    let stored = store.load().unwrap();
    assert_eq!(stored.records, vec![good.clone()]);
    assert_eq!(stored.unreadable.len(), 1);
    assert_eq!(
        stored.unreadable[0].connection_id.as_deref(),
        Some("conn-bad")
    );
    let error = store
        .usable("conn-bad", "https://relay.example", "device-a")
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ConnectionUnusable>().unwrap().reason,
        "record_unreadable"
    );
    assert!(store
        .usable("conn-good", "https://relay.example/", "device-a")
        .is_ok());
    // Another device or relay never uses this device's records.
    assert!(store
        .usable("conn-good", "https://relay.example", "device-z")
        .is_err());
    assert!(store
        .usable("conn-good", "https://other.example", "device-a")
        .is_err());

    // The relay listing it again does not add a second, readable copy.
    let stored = store
        .reconcile(
            "https://relay.example",
            "device-a",
            &[relay_connection(
                "conn-bad",
                "device-a",
                &public_b64(&secret(5)),
                "active",
            )],
        )
        .unwrap();
    assert!(stored.find("conn-bad").is_none());
    assert_eq!(stored.unreadable.len(), 1);

    // Saving keeps the unreadable entry exactly as it was.
    store.save(&stored).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(
        saved["connections"][1],
        json!({"connectionId": "conn-bad", "role": 42})
    );
}

#[test]
fn store_never_overwrites_a_file_it_cannot_parse() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.json"));
    std::fs::write(store.path(), b"{not json").unwrap();
    let stored = store.load().unwrap();
    assert!(stored.records.is_empty());
    assert_eq!(stored.unreadable.len(), 1);
    assert!(store.save(&stored).is_err());
    assert!(store
        .reconcile("https://relay.example", "device-a", &[])
        .is_err());
    let error = store
        .usable("conn-1", "https://relay.example", "device-a")
        .unwrap_err();
    assert_eq!(refusal_reason(&error), Some("store_unreadable"));
    assert_eq!(std::fs::read(store.path()).unwrap(), b"{not json");
}

#[test]
fn older_records_without_optional_fields_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.json"));
    let legacy = json!({
        "connections": [{
            "connectionId": "conn-1",
            "relayUrl": "https://relay.example",
            "localDeviceId": "device-a",
            "role": "acceptor",
            "status": "active",
            "peerUserId": "user-b",
            "peerDisplayName": "Bea",
            "peerDeviceId": "device-b",
            "peerDeviceName": "Laptop",
            "createdAt": 1,
            "fieldFromNewerBuild": {"kept": true},
        }],
    });
    std::fs::write(store.path(), legacy.to_string()).unwrap();
    let stored = store.load().unwrap();
    assert_eq!(stored.records.len(), 1);
    assert_eq!(stored.records[0].role, ConnectionRole::Acceptor);
    assert_eq!(stored.records[0].unusable_reason(), Some("not_verified"));
}

#[test]
fn reconcile_follows_status_and_disables_on_key_change() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.json"));
    let relay = "https://relay.example";
    let key_b = public_b64(&secret(6));
    let other_key = public_b64(&secret(7));

    // Pending: known, not usable, nothing pinned.
    let stored = store
        .reconcile(
            relay,
            "device-a",
            &[
                relay_connection("conn-1", "device-a", &key_b, "pending"),
                relay_connection("conn-other", "device-a2", &key_b, "active"),
            ],
        )
        .unwrap();
    assert_eq!(stored.records.len(), 1, "only this device's connections");
    assert_eq!(
        stored.records[0].unusable_reason(),
        Some("connection_pending")
    );

    // Pin as confirm does, then the relay reports active with the same key.
    let mut stored = store.load().unwrap();
    stored.records[0].pinned_public_key = Some(key_b.clone());
    store.save(&stored).unwrap();
    store
        .reconcile(
            relay,
            "device-a",
            &[relay_connection("conn-1", "device-a", &key_b, "active")],
        )
        .unwrap();
    let record = store.usable("conn-1", relay, "device-a").unwrap();
    let key = record.connection_key(&secret(8)).unwrap();
    assert_eq!(
        key,
        derive_connection_key(&secret(8), &device_crypto::public_key(&secret(6)), "conn-1")
            .unwrap()
    );

    // A different reported key disables the connection; the pin is kept.
    let stored = store
        .reconcile(
            relay,
            "device-a",
            &[relay_connection("conn-1", "device-a", &other_key, "active")],
        )
        .unwrap();
    let record = stored.find("conn-1").unwrap();
    assert_eq!(record.pinned_public_key.as_deref(), Some(key_b.as_str()));
    assert_eq!(record.unusable_reason(), Some("key_changed"));
    assert!(record.connection_key(&secret(8)).is_err());

    // Revoked by the relay: not usable, record kept.
    store
        .reconcile(
            relay,
            "device-a",
            &[relay_connection("conn-1", "device-a", &key_b, "revoked")],
        )
        .unwrap();
    assert!(store.load().unwrap().find("conn-1").is_some());
}

/// A relay that answers every request with one fixed JSON body.
async fn fake_relay(body: Value) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let body = body.to_string();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let _ = socket.read(&mut buffer).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{address}")
}

#[tokio::test]
async fn accepting_device_rejects_a_relay_that_substitutes_the_inviter_key() {
    let inviter = secret(9);
    let substituted = secret(10);
    let relay_url = fake_relay(json!({
        "inviter": {
            "user_id": "user-a",
            "display_name": "Ann",
            "device_id": "device-a",
            "device_name": "Desk",
            "public_key": public_b64(&substituted),
        },
        "expires_at": 4_000_000_000i64,
        "same_account": false,
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.json"));
    let session = AccountSession::new("token".into(), "user-b".into(), secret(11));
    let client = ConnectionsClient {
        relay_url: &relay_url,
        session: &session,
        local_device_id: "device-b",
        store: &store,
    };
    let link = InviteLink {
        relay_url: relay_url.clone(),
        code: "CODE".into(),
        fingerprint: fingerprint(&device_crypto::public_key(&inviter)).replace(' ', ""),
    };
    let error = client.accept(&link).await.unwrap_err();
    assert_eq!(refusal_reason(&error), Some("fingerprint_mismatch"));
    assert!(!store.path().exists(), "nothing is stored after a mismatch");

    // The genuine key passes the preview.
    let honest = InviteLink {
        fingerprint: fingerprint(&device_crypto::public_key(&substituted)),
        ..link.clone()
    };
    let preview = client.preview(&honest).await.unwrap();
    assert_eq!(preview.inviter.device_id, "device-a");

    // An invite made on another relay is refused before any request.
    let elsewhere = InviteLink {
        relay_url: "https://elsewhere.example".into(),
        ..link
    };
    let error = client.preview(&elsewhere).await.unwrap_err();
    assert_eq!(refusal_reason(&error), Some("different_relay"));
}
