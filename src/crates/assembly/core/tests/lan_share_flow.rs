//! Two share hosts on the loopback address: an owner shares an assistant, a
//! user finds it, asks, and is answered; access is then withdrawn, removed
//! and stopped, each side hearing of it at once.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use openbitfun_core::service::lan_share::protocol::Access;
use openbitfun_core::service::lan_share::{
    DeviceStatus, DiscoveredDevice, LanShareHost, LanShareOptions, RequestAnswer,
    ShareConfirmations, ShareSettings, SharedAssistantStore, SharedWithMeState, SharedWithMeStore,
    EVENT_REQUESTS, EVENT_SHARED_WITH_ME,
};
use openbitfun_events::EventEmitter;
use openbitfun_services_integrations::lan_share::{
    connect, ChannelConfig, DiscoveredPeer, DiscoveryEvent, LanError, ShareIdentity,
    PROTOCOL_VERSION,
};
use serde_json::{json, Value};

const WINDOW: Duration = Duration::from_millis(700);
const COOLDOWN: Duration = Duration::from_millis(900);

/// Keeps every event a host emits.
#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Events {
    fn count(&self, name: &str) -> usize {
        let events = self.0.lock().unwrap();
        events.iter().filter(|(event, _)| event == name).count()
    }
}

#[async_trait]
impl EventEmitter for Events {
    async fn emit(&self, event_name: &str, payload: Value) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((event_name.to_string(), payload));
        Ok(())
    }
}

async fn host(dir: &Path, events: Arc<Events>) -> LanShareHost {
    let assistants = dir.join("personal_assistant");
    std::fs::create_dir_all(assistants.join("workspace")).unwrap();
    LanShareHost::start(LanShareOptions {
        identity_path: dir.join("lan-share/identity.json"),
        shares: SharedAssistantStore::new(dir.join("delegation/shared-assistants.json")),
        shared_with_me: SharedWithMeStore::new(dir.join("lan-share/shared-with-me.json")),
        assistants_base: assistants,
        events,
        allow_loopback: true,
        listen_addresses: Some(vec![Ipv4Addr::LOCALHOST]),
        bonjour: false,
        request_window: WINDOW,
        cooldown: COOLDOWN,
    })
    .await
    .unwrap()
}

/// What Bonjour reports for `owner` listening on the loopback address.
async fn found(owner: &LanShareHost) -> DiscoveryEvent {
    let status = owner.status().await;
    DiscoveryEvent::Found(DiscoveredPeer {
        device_id: status.device_id,
        device_name: status.device_name,
        display_name: status.display_name,
        public_key: owner.identity().public_key(),
        fingerprint: status.fingerprint,
        protocol_version: PROTOCOL_VERSION,
        addresses: vec![Ipv4Addr::LOCALHOST],
        port: status.port.expect("the owner listens while it shares"),
    })
}

async fn eventually<F, Fut>(what: &str, check: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..250 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting until {what}");
}

fn access_of(devices: &[DiscoveredDevice], assistant_id: &str) -> Option<Access> {
    devices
        .iter()
        .flat_map(|device| device.assistants.iter())
        .find(|assistant| assistant.id == assistant_id)
        .map(|assistant| assistant.access.clone())
}

async fn my_state(user: &LanShareHost) -> Option<SharedWithMeState> {
    user.shared_with_me()
        .await
        .unwrap()
        .first()
        .map(|entry| entry.state)
}

async fn granted_keys(owner: &LanShareHost) -> Vec<String> {
    owner
        .shares()
        .share_of("workspace")
        .await
        .unwrap()
        .map(|share| {
            share
                .grants
                .into_iter()
                .map(|grant| grant.peer.public_key)
                .collect()
        })
        .unwrap_or_default()
}

/// Asks for the owner's assistant and returns the request the owner sees.
async fn ask(user: &LanShareHost, owner: &LanShareHost, owner_id: &str) -> String {
    let opened = user.request(owner_id, "workspace").await.unwrap();
    assert_eq!(opened.granted, None);
    eventually("the owner sees the request", || async {
        owner.requests().await.len() == 1
    })
    .await;
    let requests = owner.requests().await;
    assert_eq!(requests[0].request_id, opened.request_id);
    assert_eq!(requests[0].assistant_name, "Writer");
    opened.request_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_assistant_is_shared_requested_answered_withdrawn_and_stopped() {
    let owner_dir = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let owner_events = Arc::new(Events::default());
    let user_events = Arc::new(Events::default());
    let owner = host(owner_dir.path(), owner_events.clone()).await;
    let user = host(user_dir.path(), user_events.clone()).await;
    let user_key = hex::encode(user.identity().public_key());
    let owner_key = hex::encode(owner.identity().public_key());
    let owner_id = owner.identity().device_id().to_string();

    // Nothing shared: the owner does not listen.
    assert_eq!(owner.port().await, None);
    owner
        .share(
            "workspace",
            "Writer",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await
        .unwrap();
    assert!(owner.port().await.is_some());

    // Discover and read the catalog.
    user.discover_start().await.unwrap();
    user.observe(found(&owner).await).await;
    eventually("the user reads the owner's catalog", || async {
        access_of(&user.discovered(), "workspace") == Some(Access::Available)
    })
    .await;
    let devices = user.discovered();
    assert_eq!(devices[0].status, DeviceStatus::Ready);
    assert_eq!(devices[0].assistants[0].name, "Writer");

    // The request runs out and the user hears at once; asking again waits.
    ask(&user, &owner, &owner_id).await;
    assert!(owner_events.count(EVENT_REQUESTS) >= 1);
    assert!(matches!(
        access_of(&user.discovered(), "workspace"),
        Some(Access::Pending { .. })
    ));
    eventually("the expired request is declined on both sides", || async {
        owner.requests().await.is_empty()
            && matches!(
                access_of(&user.discovered(), "workspace"),
                Some(Access::Cooling { .. })
            )
    })
    .await;
    let refused = user.request(&owner_id, "workspace").await.unwrap_err();
    assert!(refused.contains("again in"), "{refused}");

    // After the cooldown the owner allows; the grant arrives by push.
    tokio::time::sleep(COOLDOWN).await;
    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Allow).await.unwrap();
    eventually("the grant reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Available)
    })
    .await;
    assert!(user_events.count(EVENT_SHARED_WITH_ME) >= 1);
    let mine = user.shared_with_me().await.unwrap();
    assert_eq!(mine[0].name, "Writer");
    assert_eq!(mine[0].owner.public_key, owner_key);
    assert_eq!(granted_keys(&owner).await, vec![user_key.clone()]);
    assert_eq!(
        access_of(&user.discovered(), "workspace"),
        Some(Access::Granted)
    );
    assert!(owner.decide(&request, RequestAnswer::Allow).await.is_err());

    // The owner withdraws access; the user hears at once.
    owner.revoke("workspace", &user_key, false).await.unwrap();
    eventually("the withdrawal reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Revoked)
    })
    .await;
    assert!(granted_keys(&owner).await.is_empty());

    // Asked again and allowed, then removed by the user.
    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Allow).await.unwrap();
    eventually("the second grant reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Available)
    })
    .await;
    user.remove(&owner_key, "workspace").await.unwrap();
    assert!(user.shared_with_me().await.unwrap().is_empty());
    eventually("the owner forgets the removed device", || async {
        granted_keys(&owner).await.is_empty()
    })
    .await;
    assert!(user
        .shared_with_me_store()
        .pending_removals()
        .await
        .unwrap()
        .is_empty());

    // Declined for good: later requests end without asking the owner.
    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Block).await.unwrap();
    tokio::time::sleep(COOLDOWN).await;
    let opened = user.request(&owner_id, "workspace").await.unwrap();
    assert_eq!(opened.granted, Some(false));
    assert!(owner.requests().await.is_empty());
    assert!(user.shared_with_me().await.unwrap().is_empty());

    // Stopping the share closes the owner to other devices.
    assert!(owner.stop("workspace").await.unwrap());
    assert_eq!(owner.port().await, None);
    user.observe(DiscoveryEvent::Lost {
        device_id: owner_id.clone(),
    })
    .await;
    assert!(user.discovered().is_empty());

    user.discover_stop().await;
    user.shutdown().await;
    owner.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_without_access_may_only_read_the_catalog_and_ask() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner = host(owner_dir.path(), Arc::new(Events::default())).await;
    owner
        .share(
            "workspace",
            "Writer",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await
        .unwrap();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, owner.port().await.unwrap()));

    let mut config = ChannelConfig::new(Arc::new(
        ShareIdentity::generate("Stranger's Mac", "Stranger").unwrap(),
    ));
    config.capabilities = vec!["assistant_share_v1".to_string()];
    config.allow_loopback = true;
    let stranger = connect(&config, address, Some(&owner.identity().public_key()), None)
        .await
        .unwrap();

    let catalog = stranger.call("catalog", json!({})).await.unwrap();
    assert_eq!(catalog["assistants"][0]["name"], "Writer");
    assert_eq!(catalog["assistants"][0]["access"]["state"], "available");

    for (method, body) in [
        ("remove", json!({ "assistantId": "workspace" })),
        ("submit", json!({})),
        ("catalog_all", json!({})),
    ] {
        assert!(
            matches!(stranger.call(method, body).await, Err(LanError::Remote(_))),
            "{method} is refused"
        );
    }
    let status = stranger
        .call("request_status", json!({ "requestId": "nope" }))
        .await
        .unwrap();
    assert_eq!(status["state"], "unknown");
    assert!(matches!(
        stranger
            .call("request", json!({ "assistantId": "workspace-missing" }))
            .await,
        Err(LanError::Remote(message)) if message.contains("not shared")
    ));

    // A device without the sharing capability cannot open a channel.
    let mut outdated =
        ChannelConfig::new(Arc::new(ShareIdentity::generate("Old Mac", "Old").unwrap()));
    outdated.allow_loopback = true;
    assert!(matches!(
        connect(&outdated, address, None, None).await,
        Err(LanError::NeedsUpdate { .. } | LanError::PeerNeedsUpdate { .. })
    ));

    stranger.close().await;
    owner.shutdown().await;
}
