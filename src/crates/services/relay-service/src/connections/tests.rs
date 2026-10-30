use super::*;
use crate::db::{connect, DeviceRow, UserRow};
use crate::MemoryAssetStore;
use axum::body::{to_bytes, Body};
use axum::http::{header, Request};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

struct World {
    app: Router,
    db: Arc<DbPool>,
    /// Alice's first device, which issues invites.
    alice: String,
    /// Another device of Alice's account.
    alice_laptop: String,
    bob: String,
    carol: String,
}

async fn world() -> World {
    let db = Arc::new(connect(":memory:").await.unwrap());
    for (user, name) in [("u-alice", "alice"), ("u-bob", "bob"), ("u-carol", "carol")] {
        UserRow::create(&db, user, name).await.unwrap();
    }
    for (device, user, name, key) in [
        ("alice-desktop", "u-alice", "Alice Desktop", "key-alice"),
        (
            "alice-laptop",
            "u-alice",
            "Alice Laptop",
            "key-alice-laptop",
        ),
        ("bob-desktop", "u-bob", "Bob Desktop", "key-bob"),
        ("carol-desktop", "u-carol", "Carol Desktop", "key-carol"),
    ] {
        DeviceRow::upsert(&db, device, user, name, Some("desktop"), Some(key))
            .await
            .unwrap();
    }
    let token = |user: &'static str, device: &'static str| {
        let db = db.clone();
        async move { AuthToken::create(&db, user, device).await.unwrap().token }
    };
    let alice = token("u-alice", "alice-desktop").await;
    let alice_laptop = token("u-alice", "alice-laptop").await;
    let bob = token("u-bob", "bob-desktop").await;
    let carol = token("u-carol", "carol-desktop").await;
    let app = crate::build_relay_router(
        Arc::new(MemoryAssetStore::new()),
        std::time::Instant::now(),
        db.clone(),
        "test",
    );
    World {
        app,
        db,
        alice,
        alice_laptop,
        bob,
        carol,
    }
}

async fn call(
    world: &World,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = world
        .app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

async fn invite(world: &World, token: &str) -> (String, String) {
    let (status, body) = call(
        world,
        "POST",
        "/api/connections/invites",
        token,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        body["invite_id"].as_str().unwrap().to_owned(),
        body["code"].as_str().unwrap().to_owned(),
    )
}

async fn accept(world: &World, token: &str, code: &str) -> (StatusCode, Value) {
    call(
        world,
        "POST",
        "/api/connections/accept",
        token,
        Some(json!({ "code": code })),
    )
    .await
}

/// Alice invites, Bob accepts; returns the pending connection id.
async fn pending_connection(world: &World) -> String {
    let (_, code) = invite(world, &world.alice).await;
    let (status, body) = accept(world, &world.bob, &code).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "pending");
    body["connection_id"].as_str().unwrap().to_owned()
}

async fn active_connection(world: &World) -> String {
    let id = pending_connection(world).await;
    let (status, body) = call(
        world,
        "POST",
        &format!("/api/connections/{id}/confirm"),
        &world.alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    id
}

async fn routed(world: &World, id: &str, user: &str, device: &str) -> Result<String, Refusal> {
    route(&world.db, id, user, device)
        .await
        .unwrap()
        .map(|route| format!("{}/{}", route.target_user_id, route.target_device_id))
}

#[tokio::test]
async fn an_invite_shows_its_sender_and_can_be_used_only_once() {
    let world = world().await;
    let (_, code) = invite(&world, &world.alice).await;

    let (status, preview) = call(
        &world,
        "POST",
        "/api/connections/invites/preview",
        &world.bob,
        Some(json!({ "code": code })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["inviter"]["display_name"], "alice");
    assert_eq!(preview["inviter"]["device_id"], "alice-desktop");
    assert_eq!(preview["inviter"]["device_name"], "Alice Desktop");
    assert_eq!(preview["inviter"]["public_key"], "key-alice");
    assert_eq!(preview["same_account"], false);

    let (status, accepted) = accept(&world, &world.bob, &code).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(accepted["role"], "acceptor");
    assert_eq!(accepted["peer"]["user_id"], "u-alice");

    for token in [&world.bob, &world.carol] {
        let (status, body) = accept(&world, token, &code).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::GONE, Some("invite_used"))
        );
    }
    let (status, body) = accept(&world, &world.carol, "no-such-code").await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("invite_not_found"))
    );
}

#[tokio::test]
async fn expired_and_withdrawn_invites_cannot_be_accepted() {
    let world = world().await;
    let (_, expired) = invite(&world, &world.alice).await;
    sqlx::query("UPDATE connection_invites SET expires_at = unixepoch() - 1 WHERE code_hash = ?")
        .bind(code_hash(&expired))
        .execute(world.db.as_ref())
        .await
        .unwrap();
    let (status, body) = accept(&world, &world.bob, &expired).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::GONE, Some("invite_expired"))
    );

    let (invite_id, withdrawn) = invite(&world, &world.alice).await;
    let path = format!("/api/connections/invites/{invite_id}");
    // Only the inviting account sees its invites.
    let (status, _) = call(&world, "DELETE", &path, &world.bob, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, listed) = call(
        &world,
        "GET",
        "/api/connections/invites",
        &world.alice_laptop,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["invite_id"] == invite_id.as_str()));
    let (status, _) = call(&world, "DELETE", &path, &world.alice, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = accept(&world, &world.bob, &withdrawn).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::GONE, Some("invite_withdrawn"))
    );

    let (used_id, used) = invite(&world, &world.alice).await;
    assert_eq!(
        accept(&world, &world.bob, &used).await.0,
        StatusCode::CREATED
    );
    let (status, body) = call(
        &world,
        "DELETE",
        &format!("/api/connections/invites/{used_id}"),
        &world.alice,
        None,
    )
    .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("invite_used"))
    );
}

#[tokio::test]
async fn invites_reject_bad_expiry_same_account_and_quota_overflow() {
    let world = world().await;
    for seconds in [0, 59, MAX_INVITE_TTL_SECS + 1] {
        let (status, body) = call(
            &world,
            "POST",
            "/api/connections/invites",
            &world.alice,
            Some(json!({ "expires_in_seconds": seconds })),
        )
        .await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_expiry"))
        );
    }
    let (_, code) = invite(&world, &world.alice).await;
    let (status, body) = accept(&world, &world.alice_laptop, &code).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::BAD_REQUEST, Some("same_account"))
    );

    for _ in 1..MAX_OPEN_INVITES_PER_ACCOUNT {
        invite(&world, &world.alice_laptop).await;
    }
    let (status, body) = call(
        &world,
        "POST",
        "/api/connections/invites",
        &world.alice,
        None,
    )
    .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("invite_quota_exceeded"))
    );
}

#[tokio::test]
async fn only_device_credentials_manage_connections() {
    let world = world().await;
    let delegated = AuthToken::create_delegated(&world.db, "u-alice", "alice-desktop")
        .await
        .unwrap()
        .token;
    let (status, body) = call(&world, "POST", "/api/connections/invites", &delegated, None).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::FORBIDDEN, Some("device_credential_required"))
    );
    let (status, _) = call(&world, "GET", "/api/connections", "not-a-token", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // A device that never registered a public key cannot be pinned.
    sqlx::query("UPDATE devices SET public_key = NULL WHERE device_id = 'carol-desktop'")
        .execute(world.db.as_ref())
        .await
        .unwrap();
    let (status, body) = call(
        &world,
        "POST",
        "/api/connections/invites",
        &world.carol,
        None,
    )
    .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("device_public_key_missing"))
    );
}

#[tokio::test]
async fn the_inviting_device_confirms_and_both_accounts_see_each_other() {
    let world = world().await;
    let id = pending_connection(&world).await;
    assert_eq!(
        routed(&world, &id, "u-bob", "bob-desktop").await,
        Err(Refusal::Pending)
    );

    let confirm = format!("/api/connections/{id}/confirm");
    for (token, expected, reason) in [
        (&world.bob, StatusCode::FORBIDDEN, "inviter_device_required"),
        (
            &world.alice_laptop,
            StatusCode::FORBIDDEN,
            "inviter_device_required",
        ),
        (&world.carol, StatusCode::NOT_FOUND, "connection_not_found"),
    ] {
        let (status, body) = call(&world, "POST", &confirm, token, None).await;
        assert_eq!((status, body["reason"].as_str()), (expected, Some(reason)));
    }
    let (status, body) = call(&world, "POST", &confirm, &world.alice, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "active");
    // Confirming twice is harmless.
    assert_eq!(
        call(&world, "POST", &confirm, &world.alice, None).await.0,
        StatusCode::OK
    );

    let (_, alice_view) = call(&world, "GET", "/api/connections", &world.alice_laptop, None).await;
    let alice_view = &alice_view[0];
    assert_eq!(alice_view["role"], "inviter");
    assert_eq!(alice_view["local_device_id"], "alice-desktop");
    assert_eq!(alice_view["peer"]["display_name"], "bob");
    assert_eq!(alice_view["peer"]["device_name"], "Bob Desktop");
    assert_eq!(alice_view["peer"]["public_key"], "key-bob");
    let (_, bob_view) = call(&world, "GET", "/api/connections", &world.bob, None).await;
    assert_eq!(bob_view[0]["role"], "acceptor");
    assert_eq!(bob_view[0]["peer"]["public_key"], "key-alice");
    let (_, carol_view) = call(&world, "GET", "/api/connections", &world.carol, None).await;
    assert_eq!(carol_view, json!([]));

    assert_eq!(
        routed(&world, &id, "u-bob", "bob-desktop").await,
        Ok("u-alice/alice-desktop".into())
    );
    assert_eq!(
        routed(&world, &id, "u-alice", "alice-desktop").await,
        Ok("u-bob/bob-desktop".into())
    );
    // Only the two paired devices are ends of the connection.
    assert_eq!(
        routed(&world, &id, "u-alice", "alice-laptop").await,
        Err(Refusal::NotFound)
    );
    assert_eq!(
        routed(&world, &id, "u-carol", "carol-desktop").await,
        Err(Refusal::NotFound)
    );
    assert_eq!(
        routed(&world, "missing", "u-bob", "bob-desktop").await,
        Err(Refusal::NotFound)
    );

    // The same two devices cannot be paired twice while one connection lives.
    let (_, code) = invite(&world, &world.alice).await;
    let (status, body) = accept(&world, &world.bob, &code).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("connection_exists"))
    );
}

#[tokio::test]
async fn either_account_revokes_and_a_third_account_cannot() {
    let world = world().await;
    let id = active_connection(&world).await;
    let path = format!("/api/connections/{id}");
    let (status, body) = call(&world, "DELETE", &path, &world.carol, None).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("connection_not_found"))
    );
    assert!(routed(&world, &id, "u-bob", "bob-desktop").await.is_ok());

    let (status, _) = call(&world, "DELETE", &path, &world.bob, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        routed(&world, &id, "u-bob", "bob-desktop").await,
        Err(Refusal::Revoked)
    );
    assert_eq!(
        routed(&world, &id, "u-alice", "alice-desktop").await,
        Err(Refusal::Revoked)
    );
    let (_, listed) = call(&world, "GET", "/api/connections", &world.alice, None).await;
    assert_eq!(listed[0]["status"], "revoked");
    assert_eq!(listed[0]["revoked_reason"], "revoked");
    let (status, body) = call(
        &world,
        "POST",
        &format!("{path}/confirm"),
        &world.alice,
        None,
    )
    .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("connection_revoked"))
    );
    assert_eq!(
        call(&world, "DELETE", &path, &world.alice_laptop, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );

    // Another device of the inviting account may revoke too.
    let second = active_connection(&world).await;
    assert_eq!(
        call(
            &world,
            "DELETE",
            &format!("/api/connections/{second}"),
            &world.alice_laptop,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        routed(&world, &second, "u-bob", "bob-desktop").await,
        Err(Refusal::Revoked)
    );
}

#[tokio::test]
async fn a_changed_device_key_or_a_removed_device_stops_routing() {
    let world = world().await;
    let id = active_connection(&world).await;
    DeviceRow::upsert(
        &world.db,
        "bob-desktop",
        "u-bob",
        "Bob Desktop",
        Some("desktop"),
        Some("key-bob-new"),
    )
    .await
    .unwrap();
    assert_eq!(
        routed(&world, &id, "u-alice", "alice-desktop").await,
        Err(Refusal::KeyChanged)
    );
    assert_eq!(
        routed(&world, &id, "u-bob", "bob-desktop").await,
        Err(Refusal::KeyChanged)
    );

    DeviceRow::delete_for_user(&world.db, "u-bob", "bob-desktop")
        .await
        .unwrap();
    assert_eq!(
        routed(&world, &id, "u-alice", "alice-desktop").await,
        Err(Refusal::Revoked)
    );
    let (_, listed) = call(&world, "GET", "/api/connections", &world.alice, None).await;
    assert_eq!(listed[0]["status"], "revoked");
    assert_eq!(listed[0]["revoked_reason"], "device_removed");
}

#[test]
fn only_delegation_commands_are_routable() {
    for command in [
        "dispatch_target_submit",
        "dispatch_target_cancel",
        "dispatch_target_workspace_bundle_commit",
    ] {
        assert!(is_routable_command(command), "{command}");
    }
    for command in [
        "host_invoke",
        "dispatch_target_",
        "dispatch_target_submit ",
        "read_stream",
        "terminal_write",
        "",
    ] {
        assert!(!is_routable_command(command), "{command}");
    }
}
