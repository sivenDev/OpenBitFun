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
    AddedDeviceStore, AddressOutcome, AddressProblem, DeviceStatus, DiscoveredDevice, LanShareHost,
    LanShareOptions, RequestAnswer, ShareConfirmations, ShareDispatch, ShareDispatchError,
    ShareSettings, SharedAssistantStore, SharedWithMeState, SharedWithMeStore, EVENT_JOB_UPDATED,
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
const RECONNECT: Duration = Duration::from_millis(200);

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
    host_with(dir, events, None).await
}

async fn host_with(
    dir: &Path,
    events: Arc<Events>,
    dispatch: Option<Arc<dyn ShareDispatch>>,
) -> LanShareHost {
    host_on(dir, events, dispatch, 0).await
}

/// A host that listens on `port` when it is free.
async fn host_on(
    dir: &Path,
    events: Arc<Events>,
    dispatch: Option<Arc<dyn ShareDispatch>>,
    port: u16,
) -> LanShareHost {
    let assistants = dir.join("personal_assistant");
    std::fs::create_dir_all(assistants.join("workspace")).unwrap();
    LanShareHost::start(LanShareOptions {
        identity_path: dir.join("lan-share/identity.json"),
        shares: SharedAssistantStore::new(dir.join("delegation/shared-assistants.json")),
        shared_with_me: SharedWithMeStore::new(dir.join("lan-share/shared-with-me.json")),
        added_devices: AddedDeviceStore::new(dir.join("lan-share/added-devices.json")),
        assistants_base: assistants,
        events,
        allow_loopback: true,
        listen_addresses: Some(vec![Ipv4Addr::LOCALHOST]),
        listen_port: port,
        reconnect_interval: RECONNECT,
        bonjour: false,
        request_window: WINDOW,
        cooldown: COOLDOWN,
        dispatch,
        dispatch_jobs: dir.join("dispatch/jobs"),
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
    assert!(owner
        .set_assistant_description("workspace", "Writes with care")
        .await
        .unwrap());

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
    assert_eq!(
        devices[0].assistants[0].description.as_deref(),
        Some("Writes with care")
    );

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

/// Claims every assistant is granted to whoever asks.
struct Boastful;

#[async_trait]
impl openbitfun_services_integrations::lan_share::RequestHandler for Boastful {
    async fn handle(
        &self,
        _peer: &openbitfun_services_integrations::lan_share::PeerIdentity,
        method: &str,
        _body: Value,
    ) -> Result<Value, String> {
        match method {
            "catalog" => Ok(json!({ "assistants": [
                { "id": "workspace", "name": "Free", "access": { "state": "granted" } }
            ] })),
            _ => Err("no".to_string()),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_alone_never_adds_an_assistant_this_device_did_not_ask_for() {
    let user_dir = tempfile::tempdir().unwrap();
    let user = host(user_dir.path(), Arc::new(Events::default())).await;
    let identity = Arc::new(ShareIdentity::generate("Lure's Mac", "Lure").unwrap());
    let mut config = ChannelConfig::new(identity.clone());
    config.capabilities = vec!["assistant_share_v1".to_string()];
    config.required_capabilities = config.capabilities.clone();
    config.allow_loopback = true;
    let listener = openbitfun_services_integrations::lan_share::LanListener::bind(
        config,
        Arc::new(Boastful),
        &[Ipv4Addr::LOCALHOST],
    )
    .await
    .unwrap();

    user.discover_start().await.unwrap();
    user.observe(DiscoveryEvent::Found(DiscoveredPeer {
        device_id: identity.device_id().to_string(),
        device_name: identity.device_name().to_string(),
        display_name: identity.display_name().to_string(),
        public_key: identity.public_key(),
        fingerprint: identity.fingerprint(),
        protocol_version: PROTOCOL_VERSION,
        addresses: vec![Ipv4Addr::LOCALHOST],
        port: listener.port(),
    }))
    .await;
    eventually("the user reads the catalog", || async {
        access_of(&user.discovered(), "workspace") == Some(Access::Granted)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(user.shared_with_me().await.unwrap().is_empty());

    user.shutdown().await;
    listener.shutdown();
}

/// Records the commands the owner's executor receives.
#[derive(Default)]
struct Executor(Mutex<Vec<(String, Value)>>);

#[async_trait]
impl ShareDispatch for Executor {
    async fn run(&self, command: &str, args: Value) -> Result<Value, String> {
        self.0
            .lock()
            .unwrap()
            .push((command.to_string(), args.clone()));
        Ok(json!({ "accepted": true, "jobId": args["jobId"], "state": "queued" }))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_with_access_runs_tasks_on_the_shared_assistant_over_the_channel() {
    let owner_dir = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let executor = Arc::new(Executor::default());
    let user_events = Arc::new(Events::default());
    let owner = host_with(
        owner_dir.path(),
        Arc::new(Events::default()),
        Some(executor.clone()),
    )
    .await;
    let user = host(user_dir.path(), user_events.clone()).await;
    let user_key = hex::encode(user.identity().public_key());
    let owner_id = owner.identity().device_id().to_string();
    owner
        .share(
            "workspace",
            "Writer",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await
        .unwrap();
    user.discover_start().await.unwrap();
    user.observe(found(&owner).await).await;
    eventually("the user reads the owner's catalog", || async {
        access_of(&user.discovered(), "workspace") == Some(Access::Available)
    })
    .await;

    // Nothing shared with this device yet: nothing is sent.
    assert_eq!(
        user.dispatch(&owner_id, "dispatch_target_probe", json!({}))
            .await
            .unwrap_err(),
        ShareDispatchError::NotShared
    );

    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Allow).await.unwrap();
    eventually("the grant reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Available)
    })
    .await;

    // A task reaches the executor bound to this device's key, whatever it claims.
    let answer = user
        .dispatch(
            &owner_id,
            "dispatch_target_submit",
            json!({
                "jobId": "job-1",
                "delegation": { "assistantId": "workspace" },
                "__sourceShareKey": "cd".repeat(32),
            }),
        )
        .await
        .unwrap();
    assert_eq!(answer["jobId"], "job-1");
    {
        let received = executor.0.lock().unwrap();
        assert_eq!(received[0].0, "dispatch_target_submit");
        assert_eq!(received[0].1["__sourceShareKey"], user_key);
    }

    // The owner tells the device when the job has news.
    let job = owner_dir.path().join("dispatch/jobs/job-1");
    std::fs::create_dir_all(&job).unwrap();
    std::fs::write(job.join("events.ndjson"), "{}\n").unwrap();
    eventually("the job update reaches the user", || async {
        user_events.count(EVENT_JOB_UPDATED) >= 1
    })
    .await;

    // Approvals and workspace commands stay with the owner.
    for command in ["dispatch_target_answer", "dispatch_target_workspace_sync"] {
        assert!(
            matches!(
                user.dispatch(&owner_id, command, json!({ "jobId": "job-1" }))
                    .await,
                Err(ShareDispatchError::Refused(_))
            ),
            "{command} is refused"
        );
    }
    assert_eq!(executor.0.lock().unwrap().len(), 1);

    // Once access is withdrawn the device knows before sending anything.
    owner.revoke("workspace", &user_key, false).await.unwrap();
    eventually("the withdrawal reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Revoked)
    })
    .await;
    assert_eq!(
        user.dispatch(
            &owner_id,
            "dispatch_target_status",
            json!({ "jobId": "job-1" })
        )
        .await
        .unwrap_err(),
        ShareDispatchError::Revoked
    );

    user.discover_stop().await;
    user.shutdown().await;
    owner.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tasks_are_refused_to_devices_without_access_and_offline_owners_are_reported() {
    let owner_dir = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let executor = Arc::new(Executor::default());
    let owner = host_with(
        owner_dir.path(),
        Arc::new(Events::default()),
        Some(executor.clone()),
    )
    .await;
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

    // A stranger's channel cannot run anything.
    let mut config = ChannelConfig::new(Arc::new(
        ShareIdentity::generate("Stranger's Mac", "Stranger").unwrap(),
    ));
    config.capabilities = vec!["assistant_share_v1".to_string()];
    config.allow_loopback = true;
    let stranger = connect(&config, address, Some(&owner.identity().public_key()), None)
        .await
        .unwrap();
    let refused = stranger
        .call(
            "dispatch",
            json!({ "command": "dispatch_target_list", "args": {} }),
        )
        .await;
    assert!(
        matches!(&refused, Err(LanError::Remote(message)) if message.contains("does not share")),
        "{refused:?}"
    );
    assert!(executor.0.lock().unwrap().is_empty());
    stranger.close().await;

    // A device granted earlier cannot reach an owner that went away.
    let user = host(user_dir.path(), Arc::new(Events::default())).await;
    let owner_id = owner.identity().device_id().to_string();
    user.discover_start().await.unwrap();
    user.observe(found(&owner).await).await;
    eventually("the user reads the owner's catalog", || async {
        access_of(&user.discovered(), "workspace") == Some(Access::Available)
    })
    .await;
    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Allow).await.unwrap();
    eventually("the grant reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Available)
    })
    .await;
    owner.shutdown().await;
    eventually("the owner's channel closes", || async {
        matches!(
            user.dispatch(&owner_id, "dispatch_target_list", json!({}))
                .await,
            Err(ShareDispatchError::Offline(_))
        )
    })
    .await;

    // Asking an owner that went away says so, and the Discover list shows it.
    let refused = user.request(&owner_id, "workspace").await.unwrap_err();
    assert!(refused.contains("not on the local network"), "{refused}");
    let devices = user.discovered();
    assert!(
        matches!(devices[0].status, DeviceStatus::Unreachable { .. }),
        "{:?}",
        devices[0].status
    );
    assert!(devices[0].assistants.is_empty());

    user.discover_stop().await;
    user.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_of_an_assistant_shared_with_this_device_stays_in_reach() {
    let owner_dir = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let owner = host(owner_dir.path(), Arc::new(Events::default())).await;
    let user_events = Arc::new(Events::default());
    let user = host(user_dir.path(), user_events.clone()).await;
    let user_key = hex::encode(user.identity().public_key());
    let owner_key = hex::encode(owner.identity().public_key());
    let owner_id = owner.identity().device_id().to_string();
    owner
        .share(
            "workspace",
            "Writer",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await
        .unwrap();

    // Granted from the Discover panel, which then closes.
    user.discover_start().await.unwrap();
    user.observe(found(&owner).await).await;
    eventually("the user reads the owner's catalog", || async {
        access_of(&user.discovered(), "workspace") == Some(Access::Available)
    })
    .await;
    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Allow).await.unwrap();
    eventually("the grant reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Available)
    })
    .await;
    user.discover_stop().await;

    // The owner stays in reach, and its news still arrives.
    assert!(user.owner_online(&owner_key));
    let now = user.shared_with_me_now().await.unwrap();
    assert!(now[0].online);
    owner.revoke("workspace", &user_key, false).await.unwrap();
    eventually(
        "the withdrawal reaches a user without the panel open",
        || async { my_state(&user).await == Some(SharedWithMeState::Revoked) },
    )
    .await;
    // Nothing usable is left, so the channel closes.
    eventually("the user lets the owner go", || async {
        !user.owner_online(&owner_key)
    })
    .await;

    // Granted again; the owner going away shows.
    user.discover_start().await.unwrap();
    user.observe(found(&owner).await).await;
    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Allow).await.unwrap();
    eventually("the second grant reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Available)
    })
    .await;
    user.discover_stop().await;
    assert!(user.owner_online(&owner_key));
    let before = user_events.count(EVENT_SHARED_WITH_ME);
    owner.shutdown().await;
    eventually("the user sees the owner offline", || async {
        !user.shared_with_me_now().await.unwrap()[0].online
    })
    .await;
    eventually("the user hears that the owner went away", || async {
        user_events.count(EVENT_SHARED_WITH_ME) > before
    })
    .await;

    user.shutdown().await;
}

fn problem_of(outcome: &AddressOutcome) -> Option<AddressProblem> {
    match outcome {
        AddressOutcome::Problem { problem, .. } => Some(*problem),
        AddressOutcome::Reached { .. } => None,
    }
}

async fn share_writer(owner: &LanShareHost) {
    owner
        .share(
            "workspace",
            "Writer",
            ShareSettings::default(),
            ShareConfirmations::default(),
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_bonjour_cannot_see_is_added_by_address_and_asked() {
    let owner_dir = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let owner = host(owner_dir.path(), Arc::new(Events::default())).await;
    let user = host(user_dir.path(), Arc::new(Events::default())).await;
    let owner_key = hex::encode(owner.identity().public_key());
    let owner_id = owner.identity().device_id().to_string();
    share_writer(&owner).await;
    let status = owner.status().await;
    let port = status.port.unwrap();
    assert_eq!(status.addresses, vec![format!("127.0.0.1:{port}")]);
    let typed = format!("127.0.0.1:{port}");

    // Typed text that is not a local address is refused before connecting.
    for (input, problem) in [
        ("", AddressProblem::Format),
        ("owner.local", AddressProblem::HostName),
        ("fd00::1", AddressProblem::Ipv6),
        ("8.8.8.8", AddressProblem::NotLocal),
    ] {
        assert_eq!(
            problem_of(&user.probe_address(input).await),
            Some(problem),
            "{input}"
        );
    }
    assert_eq!(
        problem_of(&owner.probe_address(&typed).await),
        Some(AddressProblem::ThisDevice)
    );
    // Nothing listens there.
    let closed = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert_eq!(
        problem_of(&user.probe_address(&format!("127.0.0.1:{closed}")).await),
        Some(AddressProblem::Unreachable)
    );
    // Something that is not a sharing device listens there.
    let stranger = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let stranger_port = stranger.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = stranger.accept().await {
            use tokio::io::AsyncWriteExt;
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        }
    });
    assert_eq!(
        problem_of(
            &user
                .probe_address(&format!("127.0.0.1:{stranger_port}"))
                .await
        ),
        Some(AddressProblem::NotOpenBitFun)
    );

    // The owner answers with who it is, and nothing is kept yet.
    let AddressOutcome::Reached { device } = user.probe_address(&typed).await else {
        panic!("the owner answers at its address");
    };
    assert_eq!(device.public_key, owner_key);
    assert_eq!(device.fingerprint, owner.identity().fingerprint());
    assert_eq!(device.assistants, 1);
    assert!(!device.added);
    user.discover_start().await.unwrap();
    assert!(user.discovered().is_empty());

    // Added after the codes were compared: listed and asked like any other.
    let AddressOutcome::Reached { device } = user.add_device(&typed, &owner_key).await.unwrap()
    else {
        panic!("the owner is added");
    };
    assert!(device.added);
    eventually("the added owner's catalog is read", || async {
        access_of(&user.discovered(), "workspace") == Some(Access::Available)
    })
    .await;
    let devices = user.discovered();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].added_address.as_deref(), Some(typed.as_str()));
    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Allow).await.unwrap();
    eventually("the grant reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Available)
    })
    .await;

    // Bonjour finding the same device lists it once.
    user.observe(found(&owner).await).await;
    assert_eq!(user.discovered().len(), 1);
    user.observe(DiscoveryEvent::Lost {
        device_id: owner_id.clone(),
    })
    .await;
    assert_eq!(user.discovered().len(), 1, "an added device stays listed");

    // Remembered the next time Discover opens.
    user.discover_stop().await;
    user.discover_start().await.unwrap();
    let devices = user.discovered();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].device_id, owner_id);
    user.discover_stop().await;

    // Another device now answers at that address: its identity changed.
    owner.shutdown().await;
    let impostor_dir = tempfile::tempdir().unwrap();
    let impostor = host_on(impostor_dir.path(), Arc::new(Events::default()), None, port).await;
    share_writer(&impostor).await;
    assert_eq!(impostor.port().await, Some(port));
    user.discover_start().await.unwrap();
    eventually("the changed identity shows", || async {
        matches!(
            user.discovered()
                .first()
                .map(|device| device.status.clone()),
            Some(DeviceStatus::IdentityChanged { .. })
        )
    })
    .await;
    assert_eq!(
        problem_of(&user.set_device_address(&owner_id, &typed).await.unwrap()),
        Some(AddressProblem::IdentityChanged)
    );

    // Forgotten on request; what it shared stays.
    assert!(user.remove_device(&owner_id).await.unwrap());
    assert!(user.discovered().is_empty());
    assert_eq!(my_state(&user).await, Some(SharedWithMeState::Available));
    user.discover_stop().await;
    user.discover_start().await.unwrap();
    assert!(user.discovered().is_empty());

    impostor.shutdown().await;
    user.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_owner_that_comes_back_is_reached_again_without_bonjour() {
    let owner_dir = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let owner = host(owner_dir.path(), Arc::new(Events::default())).await;
    let user = host(user_dir.path(), Arc::new(Events::default())).await;
    let owner_key = hex::encode(owner.identity().public_key());
    let user_key = hex::encode(user.identity().public_key());
    let owner_id = owner.identity().device_id().to_string();
    share_writer(&owner).await;
    let port = owner.port().await.unwrap();
    let typed = format!("127.0.0.1:{port}");

    user.discover_start().await.unwrap();
    user.add_device(&typed, &owner_key).await.unwrap();
    eventually("the added owner's catalog is read", || async {
        access_of(&user.discovered(), "workspace") == Some(Access::Available)
    })
    .await;
    let request = ask(&user, &owner, &owner_id).await;
    owner.decide(&request, RequestAnswer::Allow).await.unwrap();
    eventually("the grant reaches the user", || async {
        my_state(&user).await == Some(SharedWithMeState::Available)
    })
    .await;
    user.discover_stop().await;
    assert!(user.owner_online(&owner_key));

    // The owner goes away.
    owner.shutdown().await;
    drop(owner);
    eventually("the user sees the owner offline", || async {
        !user.owner_online(&owner_key)
    })
    .await;

    // It comes back on the same port: reached again with no Bonjour at all.
    let owner = host_on(owner_dir.path(), Arc::new(Events::default()), None, port).await;
    assert_eq!(owner.port().await, Some(port));
    eventually("the user reaches the owner again", || async {
        user.owner_online(&owner_key)
    })
    .await;
    assert!(user.shared_with_me_now().await.unwrap()[0].online);

    // Access withdrawn while the user could not hear it shows once reached.
    owner.shutdown().await;
    drop(owner);
    eventually("the user sees the owner offline again", || async {
        !user.owner_online(&owner_key)
    })
    .await;
    SharedAssistantStore::new(owner_dir.path().join("delegation/shared-assistants.json"))
        .revoke("workspace", &user_key)
        .await
        .unwrap()
        .expect("the user had access");
    let owner = host_on(owner_dir.path(), Arc::new(Events::default()), None, port).await;
    eventually("the withdrawal shows after reconnecting", || async {
        my_state(&user).await == Some(SharedWithMeState::Revoked)
    })
    .await;

    owner.shutdown().await;
    user.shutdown().await;
}
