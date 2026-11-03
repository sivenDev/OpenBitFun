//! The LAN share host of this device.
//!
//! As an owner it listens and advertises while it shares at least one
//! assistant, answers the channel methods of [`protocol`](super::protocol),
//! keeps the request queue, and pushes each decision and withdrawal to the
//! asking device at once. A device holding no grant may only read the
//! catalog, ask, and look up its own request; `remove` needs a grant.
//!
//! As a user it browses while the Discover panel is open, reads the catalog
//! of each compatible device with its advertised key pinned, asks for an
//! assistant, and keeps the grants it receives in
//! [`SharedWithMeStore`].
//!
//! Every change is announced through the host's [`EventEmitter`] as
//! `lan_share://requests`, `lan_share://shared`,
//! `lan_share://shared_with_me` and `lan_share://discovered`.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use log::{debug, info, warn};
use openbitfun_events::EventEmitter;
use openbitfun_services_core::dispatch_contract::bind_dispatch_source_share_key;
use openbitfun_services_integrations::lan_share::discovery::sharing_ipv4_addresses;
use openbitfun_services_integrations::lan_share::{
    connect, Advertisement, ChannelConfig, DiscoveredPeer, Discovery, DiscoveryEvent,
    LanConnection, LanError, LanListener, LanPush, PeerIdentity, PeerSet, RequestHandler,
    ShareIdentity,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::browse::{peer_addresses, BrowseState, DeviceStatus, DiscoveredDevice};
use super::protocol::{
    now_ms, to_local_clock, Access, AssistantBody, CatalogAssistant, CatalogResponse, Decision,
    DispatchBody, JobUpdated, RequestOpened, RequestState, RequestStatus, RequestStatusBody,
    Revoked, ASSISTANT_SHARE_CAPABILITY, COOLDOWN_SECONDS, METHOD_CATALOG, METHOD_DISPATCH,
    METHOD_REMOVE, METHOD_REQUEST, METHOD_REQUEST_STATUS, PUSH_DECISION, PUSH_JOB_UPDATED,
    PUSH_REVOKED, REQUEST_SECONDS,
};
use super::requests::{Opened, PendingRequest, RequestQueue, RequestRefusal};
use super::shared_assistants::{
    ShareConfirmations, SharePeer, ShareRefusal, ShareSettings, SharedAssistant,
    SharedAssistantStore,
};
use super::shared_with_me::{SharedWithMe, SharedWithMeState, SharedWithMeStore};
use crate::infrastructure::PathManager;

pub const EVENT_REQUESTS: &str = "lan_share://requests";
pub const EVENT_SHARED: &str = "lan_share://shared";
pub const EVENT_SHARED_WITH_ME: &str = "lan_share://shared_with_me";
pub const EVENT_DISCOVERED: &str = "lan_share://discovered";
/// A job this device started on a shared assistant has news:
/// `{ ownerKey, jobId }`.
pub const EVENT_JOB_UPDATED: &str = "lan_share://job_updated";

const LAN_SHARE_DIR: &str = "lan-share";
const IDENTITY_FILE: &str = "identity.json";
/// Connections kept per asking device; older ones are closed.
const MAX_CONNECTIONS_PER_DEVICE: usize = 4;
/// How long after a request's deadline the asking side looks it up itself
/// when no decision arrived.
const STATUS_GRACE: Duration = Duration::from_secs(2);
/// The owner's executor answers within 110 seconds; this leaves room for the
/// channel.
const DISPATCH_CALL_TIMEOUT: Duration = Duration::from_secs(125);
/// How often the owner looks for news of a job another device started.
const JOB_WATCH_INTERVAL: Duration = Duration::from_millis(500);
/// A job without news this long is no longer watched; the device still reads
/// it on its own schedule.
const JOB_WATCH_IDLE: Duration = Duration::from_secs(15 * 60);
/// Commands after which the job they name has news to watch for.
const JOB_STARTING_COMMANDS: &[&str] = &[
    "dispatch_target_submit",
    "dispatch_target_continue",
    "dispatch_target_append",
];

/// Runs Dispatch target commands for devices that use a shared assistant.
/// The arguments arrive bound to the calling device's sharing key.
#[async_trait]
pub trait ShareDispatch: Send + Sync {
    async fn run(&self, command: &str, args: Value) -> Result<Value, String>;
}

/// Why a command for a shared assistant did not reach its owner, or what the
/// owner answered instead of running it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareDispatchError {
    /// No assistant of that device is shared with this one.
    NotShared,
    /// The owner withdrew every assistant it shared with this device.
    Revoked,
    /// The owner cannot be reached on the local network.
    Offline(String),
    /// One of the two devices runs a version the other cannot talk to.
    NeedsUpdate(String),
    /// The owner refused or failed the command.
    Refused(String),
}

impl std::fmt::Display for ShareDispatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotShared => {
                formatter.write_str("no assistant of that device is shared with you")
            }
            Self::Revoked => {
                formatter.write_str("the owner stopped sharing this assistant with you")
            }
            Self::Offline(reason) => write!(
                formatter,
                "the device sharing this assistant is not on the local network: {reason}"
            ),
            Self::NeedsUpdate(reason) => write!(
                formatter,
                "update OpenBitFun on both devices to use this assistant: {reason}"
            ),
            Self::Refused(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for ShareDispatchError {}

/// Where the host keeps its state and how it reaches the network.
pub struct LanShareOptions {
    pub identity_path: PathBuf,
    pub shares: SharedAssistantStore,
    pub shared_with_me: SharedWithMeStore,
    /// Where this device's assistants live, to check a share names one.
    pub assistants_base: PathBuf,
    pub events: Arc<dyn EventEmitter>,
    /// Accept 127.0.0.1, for single-machine tools and tests.
    pub allow_loopback: bool,
    /// Addresses to listen on; `None` means this device's private ones.
    pub listen_addresses: Option<Vec<Ipv4Addr>>,
    /// Advertise and browse over Bonjour. Off in tests that feed discovery
    /// themselves.
    pub bonjour: bool,
    pub request_window: Duration,
    pub cooldown: Duration,
    /// Runs tasks for devices this one shares assistants with; `None` when
    /// this build has no executor.
    pub dispatch: Option<Arc<dyn ShareDispatch>>,
    /// Where the executor keeps jobs, to notice their news.
    pub dispatch_jobs: PathBuf,
}

impl LanShareOptions {
    pub fn for_product_home(path_manager: &PathManager, events: Arc<dyn EventEmitter>) -> Self {
        Self {
            identity_path: path_manager
                .product_home_dir()
                .join(LAN_SHARE_DIR)
                .join(IDENTITY_FILE),
            shares: SharedAssistantStore::for_product_home(path_manager),
            shared_with_me: SharedWithMeStore::for_product_home(path_manager),
            assistants_base: path_manager.assistant_workspace_base_dir(None),
            events,
            allow_loopback: false,
            listen_addresses: None,
            bonjour: true,
            request_window: Duration::from_secs(REQUEST_SECONDS),
            cooldown: Duration::from_secs(COOLDOWN_SECONDS),
            dispatch: None,
            dispatch_jobs: path_manager
                .product_home_dir()
                .join("dispatch")
                .join("jobs"),
        }
    }
}

/// This device as other devices see it, and whether it is reachable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub device_id: String,
    pub device_name: String,
    pub display_name: String,
    pub fingerprint: String,
    /// Set while this device shares an assistant and listens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Why other devices cannot see or reach this one, when they cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// A request waiting for the owner, as the confirmation dialog shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareRequestView {
    pub request_id: String,
    pub assistant_id: String,
    pub assistant_name: String,
    pub peer: SharePeer,
    pub opened_at_ms: u64,
    pub deadline_ms: u64,
}

/// The owner's answer to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestAnswer {
    Allow,
    Decline,
    /// Decline, and decline this device's later requests without asking.
    Block,
}

/// A request this device made and has no answer to yet.
#[derive(Debug, Clone)]
struct MyRequest {
    owner_key: String,
    assistant_id: String,
}

struct Inner {
    options: LanShareOptions,
    identity: Mutex<Arc<ShareIdentity>>,
    queue: Mutex<RequestQueue>,
    listener: tokio::sync::Mutex<Option<LanListener>>,
    problem: Mutex<Option<String>>,
    bonjour: Mutex<Option<Arc<Discovery>>>,
    /// Owner side: channels opened by other devices, by their key.
    incoming: Mutex<HashMap<String, Vec<LanConnection>>>,
    /// User side: channels to owners, by their key.
    owners: Mutex<HashMap<String, LanConnection>>,
    my_requests: Mutex<HashMap<String, MyRequest>>,
    browse: Mutex<BrowseState>,
    /// Whether Bonjour browsing runs: for the Discover panel, or to keep in
    /// touch with the owners of assistants shared with this device.
    browsing: AtomicBool,
    /// Whether the Discover panel is open.
    panel: AtomicBool,
    /// Whether an assistant shared with this device can be used, so its
    /// owner is looked for and its channel kept in the background.
    keep: AtomicBool,
    browser: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Owner side: jobs other devices started that are watched for news.
    watched_jobs: Mutex<HashSet<String>>,
}

/// An assistant shared with this device, and whether its owner can be
/// reached now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedWithMeNow {
    #[serde(flatten)]
    pub assistant: SharedWithMe,
    pub online: bool,
}

/// The LAN share host. Clones share one host.
#[derive(Clone)]
pub struct LanShareHost {
    inner: Arc<Inner>,
}

impl LanShareHost {
    /// Loads or creates this device's sharing identity and starts listening
    /// when an assistant is shared. A share store that cannot be read is
    /// reported by [`status`](Self::status) and shares nothing.
    pub async fn start(options: LanShareOptions) -> Result<Self, String> {
        let identity = ShareIdentity::load_or_create(&options.identity_path)
            .map_err(|error| format!("{error:#}"))?;
        let queue = RequestQueue::new(millis(options.request_window), millis(options.cooldown));
        let host = Self {
            inner: Arc::new(Inner {
                options,
                identity: Mutex::new(Arc::new(identity)),
                queue: Mutex::new(queue),
                listener: tokio::sync::Mutex::new(None),
                problem: Mutex::new(None),
                bonjour: Mutex::new(None),
                incoming: Mutex::new(HashMap::new()),
                owners: Mutex::new(HashMap::new()),
                my_requests: Mutex::new(HashMap::new()),
                browse: Mutex::new(BrowseState::default()),
                browsing: AtomicBool::new(false),
                panel: AtomicBool::new(false),
                keep: AtomicBool::new(false),
                browser: Mutex::new(None),
                watched_jobs: Mutex::new(HashSet::new()),
            }),
        };
        if let Err(error) = host.reconcile().await {
            warn!("LAN sharing not started: {}", error);
        }
        host.sync_background().await;
        Ok(host)
    }

    pub fn identity(&self) -> Arc<ShareIdentity> {
        self.inner.identity.lock().expect("identity lock").clone()
    }

    pub async fn status(&self) -> HostStatus {
        let identity = self.identity();
        HostStatus {
            device_id: identity.device_id().to_string(),
            device_name: identity.device_name().to_string(),
            display_name: identity.display_name().to_string(),
            fingerprint: identity.fingerprint(),
            port: self.port().await,
            problem: self.inner.problem.lock().expect("problem lock").clone(),
        }
    }

    /// The port this device listens on while it shares.
    pub async fn port(&self) -> Option<u16> {
        self.inner
            .listener
            .lock()
            .await
            .as_ref()
            .map(LanListener::port)
    }

    pub fn shares(&self) -> &SharedAssistantStore {
        &self.inner.options.shares
    }

    pub fn shared_with_me_store(&self) -> &SharedWithMeStore {
        &self.inner.options.shared_with_me
    }

    /// Listens and advertises while an assistant is shared; stops otherwise.
    pub async fn reconcile(&self) -> Result<(), String> {
        let shares = self.inner.options.shares.shares().await;
        let wanted = matches!(&shares, Ok(shares) if !shares.is_empty());
        let mut listener = self.inner.listener.lock().await;
        if wanted && listener.is_none() {
            let addresses = match &self.inner.options.listen_addresses {
                Some(addresses) => addresses.clone(),
                None => sharing_ipv4_addresses(self.inner.options.allow_loopback),
            };
            let handler = Arc::new(OwnerHandler {
                host: Arc::downgrade(&self.inner),
            });
            let bound = LanListener::bind(self.channel_config(), handler, &addresses)
                .await
                .map_err(|error| {
                    let problem = format!("this device cannot listen for other devices: {error}");
                    self.set_problem(Some(problem.clone()));
                    problem
                })?;
            info!(
                "LAN sharing listening: port={}, addresses={}",
                bound.port(),
                bound.addresses().len()
            );
            let port = bound.port();
            *listener = Some(bound);
            drop(listener);
            self.set_problem(self.advertise(port).err());
        } else if !wanted {
            if let Some(stopped) = listener.take() {
                stopped.shutdown();
                drop(listener);
                self.stop_advertising();
                let closing: Vec<LanConnection> = self
                    .inner
                    .incoming
                    .lock()
                    .expect("incoming lock")
                    .drain()
                    .flat_map(|(_, connections)| connections)
                    .collect();
                for connection in closing {
                    connection.close().await;
                }
                info!("LAN sharing stopped listening");
            }
            self.set_problem(shares.as_ref().err().map(ToString::to_string));
        }
        shares.map(|_| ()).map_err(|error| error.to_string())
    }

    /// Stops listening, advertising and browsing, and closes every channel.
    pub async fn shutdown(&self) {
        self.inner.panel.store(false, Ordering::SeqCst);
        self.inner.keep.store(false, Ordering::SeqCst);
        self.stop_browser().await;
        if let Some(listener) = self.inner.listener.lock().await.take() {
            listener.shutdown();
        }
        if let Some(discovery) = self.inner.bonjour.lock().expect("bonjour lock").take() {
            discovery.shutdown();
        }
        let mut closing: Vec<LanConnection> = self
            .inner
            .incoming
            .lock()
            .expect("incoming lock")
            .drain()
            .flat_map(|(_, connections)| connections)
            .collect();
        closing.extend(
            self.inner
                .owners
                .lock()
                .expect("owners lock")
                .drain()
                .map(|(_, connection)| connection),
        );
        for connection in closing {
            connection.close().await;
        }
    }

    /// Changes the name other devices see for this device.
    pub async fn set_display_name(&self, name: &str) -> Result<HostStatus, String> {
        let mut identity = (*self.identity()).clone();
        identity
            .set_display_name(name)
            .map_err(|error| error.to_string())?;
        identity
            .save(&self.inner.options.identity_path)
            .map_err(|error| format!("{error:#}"))?;
        *self.inner.identity.lock().expect("identity lock") = Arc::new(identity);
        // New channels introduce this device with the new name.
        if let Some(listener) = self.inner.listener.lock().await.take() {
            listener.shutdown();
        }
        if let Err(error) = self.reconcile().await {
            warn!("LAN sharing restart after a name change failed: {}", error);
        }
        Ok(self.status().await)
    }

    // ---- Owner ----

    /// Shares the assistant `id`, named `name` for other devices.
    pub async fn share(
        &self,
        id: &str,
        name: &str,
        settings: ShareSettings,
        confirmations: ShareConfirmations,
    ) -> Result<SharedAssistant, String> {
        let store = &self.inner.options.shares;
        store
            .share(
                &self.inner.options.assistants_base,
                id,
                settings,
                confirmations,
            )
            .await
            .map_err(|error| error.to_string())?;
        let share = store
            .set_display_name(id, name)
            .await
            .map_err(|error| error.to_string())?;
        info!("Assistant shared on the local network: assistant_id={}", id);
        let started = self.reconcile().await;
        self.emit(EVENT_SHARED, json!({})).await;
        started.map(|()| share)
    }

    pub async fn update(
        &self,
        id: &str,
        settings: ShareSettings,
        confirmations: ShareConfirmations,
    ) -> Result<SharedAssistant, String> {
        let share = self
            .inner
            .options
            .shares
            .change_settings(id, settings, confirmations)
            .await
            .map_err(|error| error.to_string())?;
        self.emit(EVENT_SHARED, json!({})).await;
        Ok(share)
    }

    /// Keeps the description other devices see for the shared assistant
    /// `id` in line with the owner's. Returns whether it changed.
    pub async fn set_assistant_description(
        &self,
        id: &str,
        description: &str,
    ) -> Result<bool, String> {
        let store = &self.inner.options.shares;
        let Some(current) = store
            .share_of(id)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        let updated = store
            .set_description(id, description)
            .await
            .map_err(|error| error.to_string())?;
        if updated.description == current.description {
            return Ok(false);
        }
        self.emit(EVENT_SHARED, json!({})).await;
        Ok(true)
    }

    /// Keeps the name other devices see for the shared assistant `id` in
    /// line with the owner's. Returns whether it changed.
    pub async fn set_assistant_name(&self, id: &str, name: &str) -> Result<bool, String> {
        let store = &self.inner.options.shares;
        let Some(current) = store
            .share_of(id)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        if current.display_name.as_deref() == Some(name.trim()) {
            return Ok(false);
        }
        store
            .set_display_name(id, name)
            .await
            .map_err(|error| error.to_string())?;
        self.emit(EVENT_SHARED, json!({})).await;
        Ok(true)
    }

    /// Stops sharing `id`: every device with access is told, and open
    /// requests for it end declined. Returns whether it was shared.
    pub async fn stop(&self, id: &str) -> Result<bool, String> {
        let removed = self
            .inner
            .options
            .shares
            .stop(id)
            .await
            .map_err(|error| error.to_string())?;
        let withdrawn = self.queue().withdraw_assistant(id, now_ms());
        for request in &withdrawn {
            self.push_decision(request, false, None).await;
        }
        if let Some(share) = &removed {
            info!(
                "Assistant no longer shared: assistant_id={}, grants={}",
                id,
                share.grants.len()
            );
            for grant in &share.grants {
                self.push_revoked(&grant.peer.public_key, id).await;
            }
        }
        if let Err(error) = self.reconcile().await {
            warn!("LAN sharing reconcile after stop failed: {}", error);
        }
        self.emit(EVENT_SHARED, json!({})).await;
        if !withdrawn.is_empty() {
            self.emit_requests().await;
        }
        Ok(removed.is_some())
    }

    /// Withdraws the access of the device with `public_key` to `id`; with
    /// `block`, its later requests are declined without asking.
    pub async fn revoke(&self, id: &str, public_key: &str, block: bool) -> Result<(), String> {
        let store = &self.inner.options.shares;
        let public_key = public_key.to_ascii_lowercase();
        let share = store
            .share_of(id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| ShareRefusal::NotShared(id.to_string()).to_string())?;
        let grant = share
            .grant(&public_key)
            .cloned()
            .ok_or_else(|| "that device has no access to this assistant".to_string())?;
        if block {
            store
                .block(id, grant.peer.clone())
                .await
                .map_err(|error| error.to_string())?;
        } else {
            store
                .revoke(id, &public_key)
                .await
                .map_err(|error| error.to_string())?;
        }
        info!(
            "Shared assistant access withdrawn: assistant_id={}, device_id={}, block={}",
            id, grant.peer.device_id, block
        );
        self.push_revoked(&public_key, id).await;
        self.emit(EVENT_SHARED, json!({})).await;
        Ok(())
    }

    /// The requests waiting for the owner, oldest first.
    pub async fn requests(&self) -> Vec<ShareRequestView> {
        self.expire_requests().await;
        self.requests_now().await
    }

    /// Answers the request `request_id`. The asking device hears at once.
    pub async fn decide(&self, request_id: &str, answer: RequestAnswer) -> Result<(), String> {
        let store = &self.inner.options.shares;
        let request = self
            .queue()
            .get(request_id)
            .cloned()
            .ok_or_else(|| "the request is no longer open".to_string())?;
        let granted = answer == RequestAnswer::Allow;
        let mut assistant = None;
        if granted {
            // Grant before closing the request, so a failed write leaves it open.
            store
                .grant(&request.assistant_id, request.peer.clone())
                .await
                .map_err(|error| error.to_string())?;
            assistant = store
                .share_of(&request.assistant_id)
                .await
                .ok()
                .flatten()
                .map(|share| CatalogAssistant {
                    id: share.id.clone(),
                    name: share_name(&share),
                    description: None,
                    access: Access::Granted,
                });
        }
        if self.queue().decide(request_id, granted, now_ms()).is_none() {
            if granted {
                // It ran out meanwhile and was declined; take the grant back.
                let _ = store
                    .revoke(&request.assistant_id, &request.peer.public_key)
                    .await;
            }
            return Err("the request is no longer open".to_string());
        }
        if answer == RequestAnswer::Block {
            store
                .block(&request.assistant_id, request.peer.clone())
                .await
                .map_err(|error| error.to_string())?;
        }
        info!(
            "Share request answered: assistant_id={}, device_id={}, answer={:?}",
            request.assistant_id, request.peer.device_id, answer
        );
        self.push_decision(&request, granted, assistant).await;
        self.emit_requests().await;
        if answer != RequestAnswer::Decline {
            self.emit(EVENT_SHARED, json!({})).await;
        }
        Ok(())
    }

    /// Declines the requests whose deadline passed.
    pub async fn expire_requests(&self) {
        let expired = self.queue().expire(now_ms());
        if expired.is_empty() {
            return;
        }
        for request in &expired {
            info!(
                "Share request ran out: assistant_id={}, device_id={}",
                request.assistant_id, request.peer.device_id
            );
            self.push_decision(request, false, None).await;
        }
        self.emit_requests().await;
    }

    async fn answer(
        &self,
        peer: &PeerIdentity,
        method: &str,
        body: Value,
    ) -> Result<Value, String> {
        let key = hex::encode(peer.public_key);
        match method {
            METHOD_CATALOG => to_value(self.catalog_for(&key).await?),
            METHOD_REQUEST => {
                let body: AssistantBody = from_value(body)?;
                to_value(self.open_request(peer, &body.assistant_id).await?)
            }
            METHOD_REQUEST_STATUS => {
                let body: RequestStatusBody = from_value(body)?;
                self.expire_requests().await;
                to_value(self.queue().status(&key, &body.request_id))
            }
            METHOD_REMOVE => {
                let body: AssistantBody = from_value(body)?;
                self.removed_by_device(&key, &body.assistant_id).await?;
                Ok(json!({}))
            }
            METHOD_DISPATCH => self.dispatch_for(&key, from_value(body)?).await,
            _ => Err(format!("unknown method {method}")),
        }
    }

    async fn catalog_for(&self, public_key: &str) -> Result<CatalogResponse, String> {
        let shares =
            self.inner.options.shares.shares().await.map_err(|_| {
                "this device cannot list its shared assistants right now".to_string()
            })?;
        let now = now_ms();
        let queue = self.queue();
        Ok(CatalogResponse {
            assistants: shares
                .iter()
                .map(|share| CatalogAssistant {
                    id: share.id.clone(),
                    name: share_name(share),
                    description: share.description.clone(),
                    access: if share.grant(public_key).is_some() {
                        Access::Granted
                    } else {
                        queue
                            .access(public_key, &share.id, now)
                            .unwrap_or(Access::Available)
                    },
                })
                .collect(),
            now_ms: now,
        })
    }

    async fn open_request(
        &self,
        peer: &PeerIdentity,
        assistant_id: &str,
    ) -> Result<RequestOpened, String> {
        let key = hex::encode(peer.public_key);
        let share = self
            .inner
            .options
            .shares
            .share_of(assistant_id)
            .await
            .map_err(|_| "this device cannot read its shared assistants right now".to_string())?
            .ok_or_else(|| ShareRefusal::NotShared(assistant_id.to_string()).to_string())?;
        let now = now_ms();
        if share.grant(&key).is_some() {
            return Ok(RequestOpened {
                request_id: uuid::Uuid::new_v4().simple().to_string(),
                deadline_ms: now,
                granted: Some(true),
                now_ms: now,
            });
        }
        let asking = share_peer(peer)?;
        let opened = self
            .queue()
            .open(asking, assistant_id, share.is_blocked(&key), now);
        match opened {
            Ok(Opened::Pending(request)) => {
                info!(
                    "Share request received: assistant_id={}, device_id={}",
                    request.assistant_id, request.peer.device_id
                );
                self.expire_at(request.deadline_ms);
                self.emit_requests().await;
                Ok(RequestOpened {
                    request_id: request.request_id,
                    deadline_ms: request.deadline_ms,
                    granted: None,
                    now_ms: now,
                })
            }
            Ok(Opened::Declined { request_id }) => {
                info!(
                    "Share request from a blocked device declined: assistant_id={}, device_id={}",
                    assistant_id,
                    share_device(&key)
                );
                Ok(RequestOpened {
                    request_id,
                    deadline_ms: now,
                    granted: Some(false),
                    now_ms: now,
                })
            }
            Err(RequestRefusal::AlreadyPending {
                request_id,
                deadline_ms,
            }) => Ok(RequestOpened {
                request_id,
                deadline_ms,
                granted: None,
                now_ms: now,
            }),
            Err(refusal) => Err(refusal.to_string()),
        }
    }

    /// The device with `public_key` removed `assistant_id` from its list.
    async fn removed_by_device(&self, public_key: &str, assistant_id: &str) -> Result<(), String> {
        let store = &self.inner.options.shares;
        let share = store
            .share_of(assistant_id)
            .await
            .map_err(|_| "this device cannot read its shared assistants right now".to_string())?
            .ok_or_else(|| ShareRefusal::NotShared(assistant_id.to_string()).to_string())?;
        if share.grant(public_key).is_none() {
            return Err(ShareRefusal::NotGranted(assistant_id.to_string()).to_string());
        }
        store
            .revoke(assistant_id, public_key)
            .await
            .map_err(|error| error.to_string())?;
        info!(
            "Shared assistant removed by its user: assistant_id={}, device_id={}",
            assistant_id,
            share_device(public_key)
        );
        self.emit(EVENT_SHARED, json!({})).await;
        Ok(())
    }

    fn expire_at(&self, deadline_ms: u64) {
        let host = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let wait = deadline_ms.saturating_sub(now_ms());
            tokio::time::sleep(Duration::from_millis(wait)).await;
            if let Some(inner) = host.upgrade() {
                LanShareHost { inner }.expire_requests().await;
            }
        });
    }

    async fn push_decision(
        &self,
        request: &PendingRequest,
        granted: bool,
        assistant: Option<CatalogAssistant>,
    ) {
        let decision = Decision {
            request_id: request.request_id.clone(),
            assistant_id: request.assistant_id.clone(),
            granted,
            assistant,
        };
        self.push_to(&request.peer.public_key, PUSH_DECISION, json!(decision))
            .await;
    }

    async fn push_revoked(&self, public_key: &str, assistant_id: &str) {
        let revoked = Revoked {
            assistant_id: assistant_id.to_string(),
        };
        self.push_to(public_key, PUSH_REVOKED, json!(revoked)).await;
    }

    /// Pushes to every open channel of the device with `public_key`. A
    /// device that is not connected learns from its next catalog.
    async fn push_to(&self, public_key: &str, topic: &str, body: Value) {
        let connections: Vec<LanConnection> = {
            let mut incoming = self.inner.incoming.lock().expect("incoming lock");
            match incoming.get_mut(public_key) {
                Some(connections) => {
                    connections.retain(|connection| !connection.is_closed());
                    connections.clone()
                }
                None => Vec::new(),
            }
        };
        let mut delivered = false;
        for connection in connections {
            delivered |= connection.push(topic, body.clone()).await.is_ok();
        }
        debug!(
            "LAN sharing push: topic={}, device_id={}, delivered={}",
            topic,
            share_device(public_key),
            delivered
        );
    }

    async fn emit_requests(&self) {
        let requests = self.requests_now().await;
        self.emit(EVENT_REQUESTS, json!({ "requests": requests }))
            .await;
    }

    async fn requests_now(&self) -> Vec<ShareRequestView> {
        let pending: Vec<PendingRequest> = self.queue().pending().to_vec();
        let shares = if pending.is_empty() {
            Vec::new()
        } else {
            self.inner.options.shares.shares().await.unwrap_or_default()
        };
        pending
            .into_iter()
            .map(|request| ShareRequestView {
                assistant_name: shares
                    .iter()
                    .find(|share| share.id == request.assistant_id)
                    .map(share_name)
                    .unwrap_or_else(|| request.assistant_id.clone()),
                request_id: request.request_id,
                assistant_id: request.assistant_id,
                peer: request.peer,
                opened_at_ms: request.opened_at_ms,
                deadline_ms: request.deadline_ms,
            })
            .collect()
    }

    // ---- User ----

    /// Opens the Discover session: browses the local network and reads the
    /// catalog of each sharing device found. Returns what is known so far.
    pub async fn discover_start(&self) -> Result<Vec<DiscoveredDevice>, String> {
        self.inner.panel.store(true, Ordering::SeqCst);
        if let Err(error) = self.start_browser() {
            self.inner.panel.store(false, Ordering::SeqCst);
            return Err(error);
        }
        // Devices found in the background are read now, and the others again:
        // the panel shows where this device stands today.
        let peers = self
            .inner
            .browse
            .lock()
            .expect("browse lock")
            .readable_peers();
        for peer in peers {
            let host = self.clone();
            tokio::spawn(async move { host.read_catalog(&peer).await });
        }
        info!("LAN sharing discovery started");
        Ok(self.discovered())
    }

    /// Starts Bonjour browsing unless it runs.
    fn start_browser(&self) -> Result<(), String> {
        if self.inner.browsing.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if self.inner.options.bonjour {
            let mut browser = match self
                .bonjour()
                .and_then(|discovery| discovery.browse().map_err(|error| format!("{error:#}")))
            {
                Ok(browser) => browser,
                Err(error) => {
                    self.inner.browsing.store(false, Ordering::SeqCst);
                    return Err(format!(
                        "cannot look for devices on the local network: {error}"
                    ));
                }
            };
            let host = Arc::downgrade(&self.inner);
            let task = tokio::spawn(async move {
                let mut peers = PeerSet::default();
                while let Some(event) = browser.next().await {
                    let Some(event) = peers.apply(event) else {
                        continue;
                    };
                    let Some(inner) = host.upgrade() else {
                        break;
                    };
                    LanShareHost { inner }.observe(event).await;
                }
            });
            *self.inner.browser.lock().expect("browser lock") = Some(task);
        }
        debug!("LAN sharing browsing started");
        Ok(())
    }

    /// Stops Bonjour browsing and closes the channels no request waits on.
    async fn stop_browser(&self) {
        if !self.inner.browsing.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Some(task) = self.inner.browser.lock().expect("browser lock").take() {
            task.abort();
        }
        self.inner.browse.lock().expect("browse lock").clear();
        self.close_idle_owners(&HashSet::new()).await;
        debug!("LAN sharing browsing stopped");
    }

    /// Closes the channels to owners that no request waits on and that are
    /// not in `kept`.
    async fn close_idle_owners(&self, kept: &HashSet<String>) {
        let waiting: Vec<String> = self
            .inner
            .my_requests
            .lock()
            .expect("requests lock")
            .values()
            .map(|request| request.owner_key.clone())
            .collect();
        let closing: Vec<LanConnection> = {
            let mut owners = self.inner.owners.lock().expect("owners lock");
            let idle: Vec<String> = owners
                .keys()
                .filter(|key| !waiting.contains(key) && !kept.contains(*key))
                .cloned()
                .collect();
            idle.iter().filter_map(|key| owners.remove(key)).collect()
        };
        for connection in closing {
            connection.close().await;
        }
    }

    /// Closes the Discover session. Channels kept for an open request stay,
    /// and so does what keeps this device in touch with the owners of
    /// assistants shared with it.
    pub async fn discover_stop(&self) {
        if !self.inner.panel.swap(false, Ordering::SeqCst) {
            return;
        }
        if self.inner.keep.load(Ordering::SeqCst) {
            let kept = self.kept_owners().await;
            self.close_idle_owners(&kept).await;
        } else {
            self.stop_browser().await;
        }
        info!("LAN sharing discovery stopped");
    }

    /// The owners of the assistants shared with this device that it can use.
    async fn kept_owners(&self) -> HashSet<String> {
        match self.inner.options.shared_with_me.assistants().await {
            Ok(assistants) => assistants
                .into_iter()
                .filter(|assistant| assistant.state == SharedWithMeState::Available)
                .map(|assistant| assistant.owner.public_key.to_ascii_lowercase())
                .collect(),
            Err(error) => {
                warn!("Assistants shared with this device not read: {}", error);
                HashSet::new()
            }
        }
    }

    /// Browses in the background while an assistant shared with this device
    /// can be used, and stops when none can and the panel is closed.
    async fn sync_background(&self) {
        let keep = !self.kept_owners().await.is_empty();
        let was = self.inner.keep.swap(keep, Ordering::SeqCst);
        if keep {
            if let Err(error) = self.start_browser() {
                warn!("LAN sharing cannot look for owners: {}", error);
            }
        } else if was && !self.inner.panel.load(Ordering::SeqCst) {
            self.stop_browser().await;
        }
    }

    /// The list of assistants shared with this device changed.
    async fn shared_with_me_changed(&self) {
        self.emit(EVENT_SHARED_WITH_ME, json!({})).await;
        self.sync_background().await;
    }

    /// Whether this device has an open channel to the owner with `owner_key`.
    pub fn owner_online(&self, owner_key: &str) -> bool {
        self.open_owner_connection(&owner_key.to_ascii_lowercase())
            .is_some()
    }

    /// The assistants shared with this device, with whether each owner can
    /// be reached now.
    pub async fn shared_with_me_now(&self) -> Result<Vec<SharedWithMeNow>, String> {
        Ok(self
            .shared_with_me()
            .await?
            .into_iter()
            .map(|assistant| SharedWithMeNow {
                online: self.owner_online(&assistant.owner.public_key),
                assistant,
            })
            .collect())
    }

    /// The devices and assistants the Discover session found.
    pub fn discovered(&self) -> Vec<DiscoveredDevice> {
        self.inner.browse.lock().expect("browse lock").devices()
    }

    /// Applies a discovery change, as Bonjour reports it. Ignored while this
    /// device does not browse.
    pub async fn observe(&self, event: DiscoveryEvent) {
        if !self.inner.browsing.load(Ordering::SeqCst) {
            return;
        }
        match event {
            DiscoveryEvent::Found(peer) => {
                if peer.device_id == self.identity().device_id() {
                    return;
                }
                let read = self
                    .inner
                    .browse
                    .lock()
                    .expect("browse lock")
                    .found(peer.clone());
                self.emit(EVENT_DISCOVERED, json!({})).await;
                // Outside the panel only the owners of assistants shared with
                // this device are reached.
                let wanted = self.inner.panel.load(Ordering::SeqCst)
                    || self
                        .kept_owners()
                        .await
                        .contains(&hex::encode(peer.public_key));
                if read && wanted {
                    let host = self.clone();
                    tokio::spawn(async move { host.read_catalog(&peer).await });
                }
            }
            DiscoveryEvent::Lost { device_id } => {
                let lost = self
                    .inner
                    .browse
                    .lock()
                    .expect("browse lock")
                    .lost(&device_id);
                if lost.is_some() {
                    self.emit(EVENT_DISCOVERED, json!({})).await;
                }
            }
        }
    }

    async fn read_catalog(&self, peer: &DiscoveredPeer) {
        let key = hex::encode(peer.public_key);
        let connection = match self
            .owner_connection(&key, &peer_addresses(peer), &peer.public_key)
            .await
        {
            Ok(connection) => connection,
            Err(error) => {
                let message = error.to_string();
                let status = match error {
                    LanError::NeedsUpdate { .. }
                    | LanError::PeerNeedsUpdate { .. }
                    | LanError::ProtocolVersion(_) => DeviceStatus::NeedsUpdate { message },
                    _ => DeviceStatus::Unreachable { message },
                };
                debug!(
                    "LAN sharing catalog unreachable: device_id={}, error={}",
                    peer.device_id, error
                );
                self.inner
                    .browse
                    .lock()
                    .expect("browse lock")
                    .failed(&peer.device_id, status);
                self.emit(EVENT_DISCOVERED, json!({})).await;
                return;
            }
        };
        self.deliver_removals(&key, &connection).await;
        self.refresh_catalog(&connection, None).await;
    }

    /// Reads the catalog of the owner at the other end of `connection` and
    /// brings the Discover list and this device's grants in line with it.
    /// A catalog never adds an assistant to this device's list by itself:
    /// only `asked`, an assistant this device just asked for, may be added.
    async fn refresh_catalog(&self, connection: &LanConnection, asked: Option<&str>) {
        let owner = connection.peer().clone();
        let catalog: Result<CatalogResponse, String> =
            match connection.call(METHOD_CATALOG, json!({})).await {
                Ok(body) => from_value(body),
                Err(error) => Err(error.to_string()),
            };
        match catalog {
            Ok(mut catalog) => {
                let local_now = now_ms();
                for assistant in &mut catalog.assistants {
                    assistant.access = assistant
                        .access
                        .clone()
                        .on_local_clock(catalog.now_ms, local_now);
                }
                self.sync_grants(&owner, &catalog.assistants, asked).await;
                self.inner
                    .browse
                    .lock()
                    .expect("browse lock")
                    .catalog(&owner.device_id, catalog.assistants);
            }
            Err(message) => {
                self.inner
                    .browse
                    .lock()
                    .expect("browse lock")
                    .failed(&owner.device_id, DeviceStatus::Unreachable { message });
            }
        }
        self.emit(EVENT_DISCOVERED, json!({})).await;
    }

    /// Records in [`SharedWithMeStore`] what the owner's catalog says about
    /// the assistants already listed, and adds `asked` when it is granted.
    async fn sync_grants(
        &self,
        owner: &PeerIdentity,
        catalog: &[CatalogAssistant],
        asked: Option<&str>,
    ) {
        let store = &self.inner.options.shared_with_me;
        let key = hex::encode(owner.public_key);
        let address = Some(owner.address.to_string());
        let result: Result<bool, String> = async {
            let mut changed = store
                .sync_with_catalog(&key, catalog, address.clone())
                .await
                .map_err(|error| error.to_string())?;
            let mine = store
                .assistants()
                .await
                .map_err(|error| error.to_string())?;
            let removing = store
                .pending_removals()
                .await
                .map_err(|error| error.to_string())?;
            for assistant in catalog {
                let listed = mine.iter().any(|entry| {
                    entry.owner.public_key == key && entry.assistant_id == assistant.id
                });
                let removed = removing.iter().any(|removal| {
                    removal.owner.public_key == key && removal.assistant_id == assistant.id
                });
                let was_asked = asked == Some(assistant.id.as_str());
                if was_asked && assistant.access == Access::Granted && !listed && !removed {
                    store
                        .add_grant(share_peer(owner)?, assistant, address.clone())
                        .await
                        .map_err(|error| error.to_string())?;
                    changed = true;
                }
            }
            Ok(changed)
        }
        .await;
        match result {
            Ok(true) => self.shared_with_me_changed().await,
            Ok(false) => {}
            Err(error) => warn!(
                "Assistants shared with this device not updated: device_id={}, error={}",
                owner.device_id, error
            ),
        }
    }

    /// Asks the device `device_id` for its assistant `assistant_id`. The
    /// answer arrives as a push; until then the assistant shows the request.
    pub async fn request(
        &self,
        device_id: &str,
        assistant_id: &str,
    ) -> Result<RequestOpened, String> {
        let peer = self
            .inner
            .browse
            .lock()
            .expect("browse lock")
            .peer(device_id)
            .cloned()
            .ok_or_else(|| "that device is no longer on the local network".to_string())?;
        let key = hex::encode(peer.public_key);
        let connection = match self
            .owner_connection(&key, &peer_addresses(&peer), &peer.public_key)
            .await
        {
            Ok(connection) => connection,
            Err(error) => {
                // The Discover list says why, next to the device.
                let problem = unreachable_owner(error);
                let status = match &problem {
                    ShareDispatchError::NeedsUpdate(message) => DeviceStatus::NeedsUpdate {
                        message: message.clone(),
                    },
                    other => DeviceStatus::Unreachable {
                        message: other.to_string(),
                    },
                };
                self.inner
                    .browse
                    .lock()
                    .expect("browse lock")
                    .failed(device_id, status);
                self.emit(EVENT_DISCOVERED, json!({})).await;
                return Err(problem.to_string());
            }
        };
        let mut opened: RequestOpened = from_value(
            connection
                .call(
                    METHOD_REQUEST,
                    json!(AssistantBody {
                        assistant_id: assistant_id.to_string(),
                    }),
                )
                .await
                .map_err(remote_message)?,
        )?;
        // From here on the deadline is on this device's clock.
        let local_now = now_ms();
        opened.deadline_ms = to_local_clock(opened.deadline_ms, opened.now_ms, local_now);
        opened.now_ms = local_now;
        match opened.granted {
            Some(granted) => {
                self.refresh_catalog(&connection, granted.then_some(assistant_id))
                    .await
            }
            None => {
                self.inner
                    .my_requests
                    .lock()
                    .expect("requests lock")
                    .insert(
                        opened.request_id.clone(),
                        MyRequest {
                            owner_key: key.clone(),
                            assistant_id: assistant_id.to_string(),
                        },
                    );
                let changed = self.inner.browse.lock().expect("browse lock").set_access(
                    &key,
                    assistant_id,
                    Access::Pending {
                        request_id: opened.request_id.clone(),
                        deadline_ms: opened.deadline_ms,
                    },
                );
                if changed {
                    self.emit(EVENT_DISCOVERED, json!({})).await;
                }
                self.look_up_after(connection, opened.clone());
            }
        }
        Ok(opened)
    }

    /// Looks the request up itself when no decision arrived by its deadline,
    /// such as after the channel dropped.
    fn look_up_after(&self, connection: LanConnection, opened: RequestOpened) {
        let host = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let wait = Duration::from_millis(opened.deadline_ms.saturating_sub(now_ms()));
            tokio::time::sleep(wait + STATUS_GRACE).await;
            let Some(inner) = host.upgrade() else {
                return;
            };
            let host = LanShareHost { inner };
            let open = host
                .inner
                .my_requests
                .lock()
                .expect("requests lock")
                .get(&opened.request_id)
                .cloned();
            let Some(request) = open else {
                return;
            };
            let status: Result<RequestStatus, String> = match connection
                .call(
                    METHOD_REQUEST_STATUS,
                    json!(RequestStatusBody {
                        request_id: opened.request_id.clone(),
                    }),
                )
                .await
            {
                Ok(body) => from_value(body),
                Err(error) => Err(error.to_string()),
            };
            let granted = match status {
                Ok(status) if status.state == RequestState::Pending => {
                    host.look_up_after(connection, opened);
                    return;
                }
                Ok(status) => status.state == RequestState::Granted,
                Err(error) => {
                    debug!(
                        "Share request status unavailable: assistant_id={}, error={}",
                        request.assistant_id, error
                    );
                    false
                }
            };
            host.inner
                .my_requests
                .lock()
                .expect("requests lock")
                .remove(&opened.request_id);
            if !connection.is_closed() {
                let asked = granted.then_some(request.assistant_id.as_str());
                host.refresh_catalog(&connection, asked).await;
            }
        });
    }

    /// The assistants other devices shared with this one.
    pub async fn shared_with_me(&self) -> Result<Vec<SharedWithMe>, String> {
        self.inner
            .options
            .shared_with_me
            .assistants()
            .await
            .map_err(|error| error.to_string())
    }

    /// Removes `assistant_id` of the owner with `owner_key` from this
    /// device. The owner hears of it now, or the next time it is found.
    pub async fn remove(&self, owner_key: &str, assistant_id: &str) -> Result<(), String> {
        let owner_key = owner_key.to_ascii_lowercase();
        let removed = self
            .inner
            .options
            .shared_with_me
            .remove(&owner_key, assistant_id, true)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "that assistant is not in your list".to_string())?;
        info!(
            "Shared assistant removed from this device: assistant_id={}, owner_device_id={}",
            assistant_id, removed.owner.device_id
        );
        self.shared_with_me_changed().await;
        let addresses = self.owner_addresses(&owner_key, removed.last_address.as_deref());
        let Some(public_key) = key_bytes(&owner_key) else {
            return Ok(());
        };
        match self
            .owner_connection(&owner_key, &addresses, &public_key)
            .await
        {
            Ok(connection) => {
                self.deliver_removals(&owner_key, &connection).await;
                if self.inner.panel.load(Ordering::SeqCst) {
                    self.refresh_catalog(&connection, None).await;
                }
            }
            Err(error) => debug!(
                "Removal kept until the owner is reachable: assistant_id={}, error={}",
                assistant_id, error
            ),
        }
        Ok(())
    }

    /// Tells the owner at the other end of `connection` of the removals it
    /// has not heard of.
    async fn deliver_removals(&self, owner_key: &str, connection: &LanConnection) {
        let store = &self.inner.options.shared_with_me;
        let Ok(removals) = store.pending_removals().await else {
            return;
        };
        for removal in removals
            .iter()
            .filter(|removal| removal.owner.public_key == owner_key)
        {
            let delivered = match connection
                .call(
                    METHOD_REMOVE,
                    json!(AssistantBody {
                        assistant_id: removal.assistant_id.clone(),
                    }),
                )
                .await
            {
                Ok(_) => true,
                // The owner no longer shares it, or already withdrew access.
                Err(LanError::Remote(_)) => true,
                Err(error) => {
                    debug!(
                        "Removal not delivered yet: assistant_id={}, error={}",
                        removal.assistant_id, error
                    );
                    false
                }
            };
            if delivered {
                if let Err(error) = store
                    .removal_delivered(owner_key, &removal.assistant_id)
                    .await
                {
                    warn!("Delivered removal not recorded: {}", error);
                }
            }
        }
    }

    /// A channel to the owner with `owner_key`, reusing an open one. The
    /// owner must hold exactly that key.
    async fn owner_connection(
        &self,
        owner_key: &str,
        addresses: &[SocketAddr],
        public_key: &[u8; 32],
    ) -> Result<LanConnection, LanError> {
        if let Some(open) = self
            .inner
            .owners
            .lock()
            .expect("owners lock")
            .get(owner_key)
            .filter(|connection| !connection.is_closed())
        {
            return Ok(open.clone());
        }
        let config = self.channel_config();
        let mut failure =
            LanError::Io("the device has no address on the local network".to_string());
        for address in addresses {
            match connect(&config, *address, Some(public_key), None).await {
                Ok(connection) => {
                    let kept = {
                        let mut owners = self.inner.owners.lock().expect("owners lock");
                        match owners.get(owner_key).filter(|open| !open.is_closed()) {
                            Some(open) => Err(open.clone()),
                            None => {
                                owners.insert(owner_key.to_string(), connection.clone());
                                Ok(connection.clone())
                            }
                        }
                    };
                    return match kept {
                        Ok(connection) => {
                            self.watch_pushes(&connection);
                            if self.inner.keep.load(Ordering::SeqCst) {
                                // The owner can be reached again.
                                self.emit(EVENT_SHARED_WITH_ME, json!({})).await;
                            }
                            Ok(connection)
                        }
                        Err(open) => {
                            connection.close().await;
                            Ok(open)
                        }
                    };
                }
                Err(
                    error @ (LanError::NeedsUpdate { .. }
                    | LanError::PeerNeedsUpdate { .. }
                    | LanError::ProtocolVersion(_)
                    | LanError::PeerKeyMismatch),
                ) => return Err(error),
                Err(error) => failure = error,
            }
        }
        Err(failure)
    }

    fn watch_pushes(&self, connection: &LanConnection) {
        let Some(mut pushes) = connection.take_pushes() else {
            return;
        };
        let owner = connection.peer().clone();
        let host = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            while let Some(push) = pushes.recv().await {
                let Some(inner) = host.upgrade() else {
                    break;
                };
                LanShareHost { inner }.on_push(&owner, push).await;
            }
        });
        // The channel closed: the owner may have gone away.
        let closing = connection.clone();
        let owner = closing.peer().clone();
        let host = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            closing.closed().await;
            if let Some(inner) = host.upgrade() {
                let host = LanShareHost { inner };
                let key = hex::encode(owner.public_key);
                host.inner
                    .owners
                    .lock()
                    .expect("owners lock")
                    .retain(|_, open| !open.is_closed());
                debug!(
                    "LAN sharing channel to an owner closed: device_id={}",
                    owner.device_id
                );
                if host.kept_owners().await.contains(&key) {
                    host.emit(EVENT_SHARED_WITH_ME, json!({})).await;
                }
            }
        });
    }

    async fn on_push(&self, owner: &PeerIdentity, push: LanPush) {
        let key = hex::encode(owner.public_key);
        match push.topic.as_str() {
            PUSH_DECISION => {
                let Ok(decision) = serde_json::from_value::<Decision>(push.body) else {
                    debug!("Malformed share decision ignored");
                    return;
                };
                let mine = self
                    .inner
                    .my_requests
                    .lock()
                    .expect("requests lock")
                    .remove(&decision.request_id);
                // Only answers to this device's own requests count.
                let Some(request) = mine.filter(|request| {
                    request.owner_key == key && request.assistant_id == decision.assistant_id
                }) else {
                    debug!("Share decision for an unknown request ignored");
                    return;
                };
                info!(
                    "Share request answered by its owner: assistant_id={}, granted={}",
                    request.assistant_id, decision.granted
                );
                if decision.granted {
                    let assistant = decision.assistant.unwrap_or(CatalogAssistant {
                        id: request.assistant_id.clone(),
                        name: request.assistant_id.clone(),
                        description: None,
                        access: Access::Granted,
                    });
                    let added = match share_peer(owner) {
                        Ok(peer) => self
                            .inner
                            .options
                            .shared_with_me
                            .add_grant(peer, &assistant, Some(owner.address.to_string()))
                            .await
                            .map_err(|error| error.to_string()),
                        Err(error) => Err(error),
                    };
                    match added {
                        Ok(_) => self.shared_with_me_changed().await,
                        Err(error) => warn!("Granted assistant not recorded: {}", error),
                    }
                    let changed = self.inner.browse.lock().expect("browse lock").set_access(
                        &key,
                        &request.assistant_id,
                        Access::Granted,
                    );
                    if changed {
                        self.emit(EVENT_DISCOVERED, json!({})).await;
                    }
                } else if let Some(connection) = self.open_owner_connection(&key) {
                    // The owner's catalog says how long to wait before asking again.
                    self.refresh_catalog(&connection, None).await;
                }
            }
            PUSH_JOB_UPDATED => {
                let Ok(update) = serde_json::from_value::<JobUpdated>(push.body) else {
                    debug!("Malformed job update ignored");
                    return;
                };
                self.emit(
                    EVENT_JOB_UPDATED,
                    json!({ "ownerKey": key, "jobId": update.job_id }),
                )
                .await;
            }
            PUSH_REVOKED => {
                let Ok(revoked) = serde_json::from_value::<Revoked>(push.body) else {
                    debug!("Malformed share withdrawal ignored");
                    return;
                };
                info!(
                    "Shared assistant withdrawn by its owner: assistant_id={}",
                    revoked.assistant_id
                );
                match self
                    .inner
                    .options
                    .shared_with_me
                    .mark_revoked(&key, &revoked.assistant_id)
                    .await
                {
                    Ok(true) => self.shared_with_me_changed().await,
                    Ok(false) => {}
                    Err(error) => warn!("Withdrawn assistant not recorded: {}", error),
                }
                if let Some(connection) = self.open_owner_connection(&key) {
                    if self.inner.panel.load(Ordering::SeqCst) {
                        self.refresh_catalog(&connection, None).await;
                    }
                }
            }
            other => debug!("Unknown LAN sharing push ignored: topic={}", other),
        }
    }

    // ---- Tasks for shared assistants ----

    /// Owner side: runs a Dispatch command from the device with `public_key`,
    /// which must hold one of this device's shared assistants. The executor
    /// decides from the bound key which assistant and jobs it may touch.
    async fn dispatch_for(&self, public_key: &str, body: DispatchBody) -> Result<Value, String> {
        let shares =
            self.inner.options.shares.shares().await.map_err(|_| {
                "this device cannot list its shared assistants right now".to_string()
            })?;
        if !shares.iter().any(|share| share.grant(public_key).is_some()) {
            return Err("this device does not share an assistant with you".to_string());
        }
        let executor = self.inner.options.dispatch.clone().ok_or_else(|| {
            "this device cannot run tasks for its shared assistants: its task runner is missing"
                .to_string()
        })?;
        let args = bind_dispatch_source_share_key(&body.command, body.args, public_key)?;
        let asked_job = args
            .get("jobId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let answer = executor.run(&body.command, args).await?;
        if JOB_STARTING_COMMANDS.contains(&body.command.as_str()) {
            let job_id = answer
                .get("jobId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(asked_job);
            if let Some(job_id) = job_id {
                self.watch_job(public_key, &job_id);
            }
        }
        Ok(answer)
    }

    /// Owner side: tells the device with `public_key` whenever job `job_id`
    /// has news, until it has none for a while.
    fn watch_job(&self, public_key: &str, job_id: &str) {
        if !is_job_id(job_id) {
            return;
        }
        let watch_key = format!("{public_key}/{job_id}");
        if !self
            .inner
            .watched_jobs
            .lock()
            .expect("watched jobs lock")
            .insert(watch_key.clone())
        {
            return;
        }
        let job_dir = self.inner.options.dispatch_jobs.join(job_id);
        let public_key = public_key.to_string();
        let job_id = job_id.to_string();
        let host = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut seen = job_mark(&job_dir);
            let mut quiet = Duration::ZERO;
            while quiet < JOB_WATCH_IDLE {
                tokio::time::sleep(JOB_WATCH_INTERVAL).await;
                let Some(inner) = host.upgrade() else {
                    return;
                };
                let mark = job_mark(&job_dir);
                if mark == seen {
                    quiet += JOB_WATCH_INTERVAL;
                    continue;
                }
                seen = mark;
                quiet = Duration::ZERO;
                LanShareHost { inner }
                    .push_to_device(
                        &public_key,
                        PUSH_JOB_UPDATED,
                        json!(JobUpdated {
                            job_id: job_id.clone(),
                        }),
                    )
                    .await;
            }
            if let Some(inner) = host.upgrade() {
                inner
                    .watched_jobs
                    .lock()
                    .expect("watched jobs lock")
                    .remove(&watch_key);
            }
        });
    }

    /// Owner side: pushes to every open channel of the device with `public_key`.
    async fn push_to_device(&self, public_key: &str, topic: &str, body: Value) {
        let connections = self
            .inner
            .incoming
            .lock()
            .expect("incoming lock")
            .get(public_key)
            .map(|connections| {
                connections
                    .iter()
                    .filter(|connection| !connection.is_closed())
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for connection in connections {
            if let Err(error) = connection.push(topic, body.clone()).await {
                debug!(
                    "Push to a device not delivered: topic={}, error={}",
                    topic, error
                );
            }
        }
    }

    /// User side: sends a Dispatch target command to the device `device_id`,
    /// which shares an assistant with this one.
    pub async fn dispatch(
        &self,
        device_id: &str,
        command: &str,
        args: Value,
    ) -> Result<Value, ShareDispatchError> {
        let entries = self
            .inner
            .options
            .shared_with_me
            .assistants()
            .await
            .map_err(|error| ShareDispatchError::Refused(error.to_string()))?;
        let from_owner: Vec<&SharedWithMe> = entries
            .iter()
            .filter(|entry| entry.owner.device_id == device_id)
            .collect();
        let Some(entry) = from_owner
            .iter()
            .find(|entry| entry.state == SharedWithMeState::Available)
            .copied()
        else {
            return Err(if from_owner.is_empty() {
                ShareDispatchError::NotShared
            } else {
                ShareDispatchError::Revoked
            });
        };
        let owner_key = entry.owner.public_key.clone();
        let public_key = key_bytes(&owner_key).ok_or(ShareDispatchError::NotShared)?;
        let addresses = self.owner_addresses(&owner_key, entry.last_address.as_deref());
        let connection = self
            .owner_connection(&owner_key, &addresses, &public_key)
            .await
            .map_err(unreachable_owner)?;
        let call = connection.call(
            METHOD_DISPATCH,
            json!(DispatchBody {
                command: command.to_string(),
                args,
            }),
        );
        match tokio::time::timeout(DISPATCH_CALL_TIMEOUT, call).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(LanError::Remote(message))) => Err(ShareDispatchError::Refused(message)),
            Ok(Err(error)) => Err(unreachable_owner(error)),
            Err(_) => Err(ShareDispatchError::Offline(
                "the owner did not answer in time".to_string(),
            )),
        }
    }

    /// Where the owner with `owner_key` may be: what Bonjour lists now, then
    /// where it was last reached.
    fn owner_addresses(&self, owner_key: &str, last_address: Option<&str>) -> Vec<SocketAddr> {
        let mut addresses: Vec<SocketAddr> = self
            .inner
            .browse
            .lock()
            .expect("browse lock")
            .peer_with_key(owner_key)
            .map(peer_addresses)
            .unwrap_or_default();
        if let Some(last) = last_address.and_then(|address| address.parse().ok()) {
            if !addresses.contains(&last) {
                addresses.push(last);
            }
        }
        addresses
    }

    fn open_owner_connection(&self, owner_key: &str) -> Option<LanConnection> {
        self.inner
            .owners
            .lock()
            .expect("owners lock")
            .get(owner_key)
            .filter(|connection| !connection.is_closed())
            .cloned()
    }

    // ---- Shared ----

    fn channel_config(&self) -> ChannelConfig {
        let mut config = ChannelConfig::new(self.identity());
        config.capabilities = vec![ASSISTANT_SHARE_CAPABILITY.to_string()];
        config.required_capabilities = config.capabilities.clone();
        config.allow_loopback = self.inner.options.allow_loopback;
        config
    }

    fn queue(&self) -> std::sync::MutexGuard<'_, RequestQueue> {
        self.inner.queue.lock().expect("queue lock")
    }

    fn set_problem(&self, problem: Option<String>) {
        *self.inner.problem.lock().expect("problem lock") = problem;
    }

    fn bonjour(&self) -> Result<Arc<Discovery>, String> {
        let mut bonjour = self.inner.bonjour.lock().expect("bonjour lock");
        if let Some(discovery) = bonjour.as_ref() {
            return Ok(discovery.clone());
        }
        let discovery = Arc::new(
            Discovery::new(
                self.identity().device_id(),
                self.inner.options.allow_loopback,
            )
            .map_err(|error| format!("{error:#}"))?,
        );
        *bonjour = Some(discovery.clone());
        Ok(discovery)
    }

    fn advertise(&self, port: u16) -> Result<(), String> {
        if !self.inner.options.bonjour {
            return Ok(());
        }
        let identity = self.identity();
        let advertisement = Advertisement {
            device_id: identity.device_id().to_string(),
            device_name: identity.device_name().to_string(),
            display_name: identity.display_name().to_string(),
            public_key: identity.public_key(),
            port,
        };
        self.bonjour()
            .and_then(|discovery| {
                discovery
                    .advertise(&advertisement)
                    .map_err(|error| format!("{error:#}"))
            })
            .map_err(|error| {
                warn!("LAN sharing advertisement failed: {}", error);
                format!("other devices cannot see this device: {error}")
            })
    }

    fn stop_advertising(&self) {
        if let Some(discovery) = self.inner.bonjour.lock().expect("bonjour lock").as_ref() {
            if let Err(error) = discovery.stop_advertising() {
                debug!("LAN sharing advertisement not withdrawn: {:#}", error);
            }
        }
    }

    async fn emit(&self, event: &str, payload: Value) {
        if let Err(error) = self.inner.options.events.emit(event, payload).await {
            debug!(
                "LAN sharing event not delivered: event={}, error={}",
                event, error
            );
        }
    }
}

/// Answers the channel methods of other devices.
struct OwnerHandler {
    host: Weak<Inner>,
}

#[async_trait]
impl RequestHandler for OwnerHandler {
    async fn handle(
        &self,
        peer: &PeerIdentity,
        method: &str,
        body: Value,
    ) -> Result<Value, String> {
        let inner = self
            .host
            .upgrade()
            .ok_or_else(|| "this device stopped sharing".to_string())?;
        LanShareHost { inner }.answer(peer, method, body).await
    }

    async fn connected(&self, connection: LanConnection) {
        let Some(inner) = self.host.upgrade() else {
            connection.close().await;
            return;
        };
        // Devices only answer here; drain anything they push.
        if let Some(mut pushes) = connection.take_pushes() {
            tokio::spawn(async move { while pushes.recv().await.is_some() {} });
        }
        let key = hex::encode(connection.peer().public_key);
        let evicted = {
            let mut incoming = inner.incoming.lock().expect("incoming lock");
            let connections = incoming.entry(key).or_default();
            connections.retain(|open| !open.is_closed());
            connections.push(connection);
            let excess = connections.len().saturating_sub(MAX_CONNECTIONS_PER_DEVICE);
            connections.drain(..excess).collect::<Vec<_>>()
        };
        for connection in evicted {
            connection.close().await;
        }
    }
}

fn share_name(share: &SharedAssistant) -> String {
    share
        .display_name
        .clone()
        .unwrap_or_else(|| share.id.clone())
}

fn share_peer(peer: &PeerIdentity) -> Result<SharePeer, String> {
    SharePeer::new(
        &hex::encode(peer.public_key),
        &peer.fingerprint,
        &peer.display_name,
        &peer.device_name,
    )
}

fn share_device(public_key: &str) -> String {
    super::shared_assistants::share_device_id(public_key).unwrap_or_default()
}

fn key_bytes(public_key: &str) -> Option<[u8; 32]> {
    hex::decode(public_key).ok()?.try_into().ok()
}

fn unreachable_owner(error: LanError) -> ShareDispatchError {
    match error {
        LanError::NeedsUpdate { .. }
        | LanError::PeerNeedsUpdate { .. }
        | LanError::ProtocolVersion(_) => ShareDispatchError::NeedsUpdate(error.to_string()),
        other => ShareDispatchError::Offline(other.to_string()),
    }
}

/// A job id the executor would accept, so it never names another path.
fn is_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && job_id.len() <= 128
        && job_id != "."
        && job_id != ".."
        && job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// What changes when a job has news: its event log grows or its record is
/// rewritten.
fn job_mark(job_dir: &Path) -> (u64, Option<std::time::SystemTime>) {
    let events = std::fs::metadata(job_dir.join("events.ndjson"))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let record = std::fs::metadata(job_dir.join("job.json"))
        .and_then(|metadata| metadata.modified())
        .ok();
    (events, record)
}

fn remote_message(error: LanError) -> String {
    match error {
        LanError::Remote(message) => message,
        other => other.to_string(),
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn to_value<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn from_value<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("malformed message: {error}"))
}
