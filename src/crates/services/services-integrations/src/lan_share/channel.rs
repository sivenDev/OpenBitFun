//! The direct, end-to-end encrypted channel between two sharing devices.
//!
//! A connection is a WebSocket over TCP. After the handshake in
//! [`super::handshake`] both sides may send requests, answer them, and push
//! messages. Listeners bind only private network addresses and refuse
//! connections from anywhere else; nothing reaches a request handler before
//! the handshake has finished.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch, Semaphore};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::WebSocketStream;

use super::handshake::{CheckedHello, FrameOpener, FrameSealer, Hello, LocalHandshake};
use super::identity::{fingerprint_of, ShareIdentity};
use super::{is_sharing_address, PROTOCOL_VERSION};

const CLOSE_NEEDS_UPDATE: u16 = 4001;
const CLOSE_PROTOCOL_VERSION: u16 = 4002;
const CLOSE_KEY_MISMATCH: u16 = 4003;
/// Room for the frame counter, the AEAD tag, and the envelope fields.
const FRAME_OVERHEAD: usize = 256;
/// Requests one connection may have in a handler at once.
const MAX_INFLIGHT_REQUESTS: usize = 32;
const OUTGOING_QUEUE: usize = 64;
const PUSH_QUEUE: usize = 64;
const BIND_ATTEMPTS: usize = 5;

/// How a device takes part in channels.
#[derive(Clone)]
pub struct ChannelConfig {
    pub identity: Arc<ShareIdentity>,
    /// Capabilities this device offers.
    pub capabilities: Vec<String>,
    /// Capabilities a peer must offer, or the connection ends as needing an
    /// update.
    pub required_capabilities: Vec<String>,
    pub handshake_timeout: Duration,
    /// Largest message body this device sends or accepts.
    pub max_frame_bytes: usize,
    /// Connections that may be in their handshake at once.
    pub max_pending_handshakes: usize,
    /// Accept loopback addresses; only tests and single-machine tools set it.
    pub allow_loopback: bool,
    /// The port a listener tries first on every address, so devices that
    /// cannot see each other over Bonjour can connect by address alone. When
    /// it is taken anywhere, or 0, the listener takes any free port.
    pub listen_port: u16,
}

impl ChannelConfig {
    pub fn new(identity: Arc<ShareIdentity>) -> Self {
        Self {
            identity,
            capabilities: Vec::new(),
            required_capabilities: Vec::new(),
            handshake_timeout: Duration::from_secs(5),
            max_frame_bytes: 4 * 1024 * 1024,
            max_pending_handshakes: 16,
            allow_loopback: false,
            listen_port: 0,
        }
    }

    fn websocket(&self) -> WebSocketConfig {
        let limit = self.max_frame_bytes + FRAME_OVERHEAD;
        WebSocketConfig::default()
            .max_message_size(Some(limit))
            .max_frame_size(Some(limit))
    }
}

/// Why a channel could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LanError {
    /// The peer requires capabilities this device does not offer.
    #[error("this device needs an update to talk to the other device (missing: {})", .missing.join(", "))]
    NeedsUpdate { missing: Vec<String> },
    /// The peer lacks capabilities this device requires.
    #[error("the other device needs an update (missing: {})", .missing.join(", "))]
    PeerNeedsUpdate { missing: Vec<String> },
    #[error("the other device speaks protocol version {0}, which this version does not support")]
    ProtocolVersion(u32),
    #[error("the other device's key does not match the one it advertised")]
    PeerKeyMismatch,
    #[error("the other device did not answer in time")]
    Timeout,
    /// Nothing accepted a connection at the address: the device is off, not
    /// listening, or a proxy, VPN or firewall is in the way.
    #[error("the other device cannot be reached: {0}")]
    Unreachable(String),
    #[error("the connection was refused: {0}")]
    Refused(String),
    #[error("the connection is closed")]
    Closed,
    #[error("the other device returned an error: {0}")]
    Remote(String),
    #[error("the connection failed: {0}")]
    Io(String),
    #[error("the other device broke the protocol: {0}")]
    Protocol(String),
    #[error("the message is larger than the {0}-byte limit")]
    TooLarge(usize),
}

/// The checked identity of the device at the other end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerIdentity {
    pub device_id: String,
    pub device_name: String,
    pub display_name: String,
    pub public_key: [u8; 32],
    pub fingerprint: String,
    pub capabilities: Vec<String>,
    pub address: SocketAddr,
}

/// A message pushed by the other device.
#[derive(Debug, Clone, PartialEq)]
pub struct LanPush {
    pub topic: String,
    pub body: Value,
}

/// Answers requests from the other end of a channel.
#[async_trait]
pub trait RequestHandler: Send + Sync + 'static {
    async fn handle(&self, peer: &PeerIdentity, method: &str, body: Value)
        -> Result<Value, String>;

    /// A peer finished the handshake; the connection lets this side call or
    /// push back.
    async fn connected(&self, _connection: LanConnection) {}
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Envelope {
    Ready,
    Request {
        id: u64,
        method: String,
        body: Value,
    },
    Response {
        id: u64,
        #[serde(default)]
        body: Value,
        #[serde(default)]
        error: Option<String>,
    },
    Push {
        topic: String,
        body: Value,
    },
}

enum Outgoing {
    Frame(Vec<u8>),
    Close,
}

type Pending = std::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value, LanError>>>>;

struct Inner {
    peer: PeerIdentity,
    outgoing: mpsc::Sender<Outgoing>,
    pending: Pending,
    next_id: AtomicU64,
    closed: watch::Receiver<bool>,
    pushes: std::sync::Mutex<Option<mpsc::Receiver<LanPush>>>,
    max_frame_bytes: usize,
}

/// An established channel. Clones share the same connection.
#[derive(Clone)]
pub struct LanConnection {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for LanConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanConnection")
            .field("peer", &self.inner.peer.device_id)
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl LanConnection {
    pub fn peer(&self) -> &PeerIdentity {
        &self.inner.peer
    }

    /// Sends a request and waits for its answer.
    pub async fn call(&self, method: &str, body: Value) -> Result<Value, LanError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .expect("pending lock")
            .insert(id, sender);
        let sent = self
            .send(&Envelope::Request {
                id,
                method: method.to_string(),
                body,
            })
            .await;
        if let Err(error) = sent {
            self.inner.pending.lock().expect("pending lock").remove(&id);
            return Err(error);
        }
        receiver.await.unwrap_or(Err(LanError::Closed))
    }

    /// Pushes a message the other device receives through its push stream.
    pub async fn push(&self, topic: &str, body: Value) -> Result<(), LanError> {
        self.send(&Envelope::Push {
            topic: topic.to_string(),
            body,
        })
        .await
    }

    /// The pushes from the other device; available once.
    pub fn take_pushes(&self) -> Option<mpsc::Receiver<LanPush>> {
        self.inner.pushes.lock().expect("push lock").take()
    }

    pub fn is_closed(&self) -> bool {
        *self.inner.closed.borrow()
    }

    /// Waits until the connection has closed.
    pub async fn closed(&self) {
        let mut closed = self.inner.closed.clone();
        let _ = closed.wait_for(|closed| *closed).await;
    }

    /// Closes the connection.
    pub async fn close(&self) {
        let _ = self.inner.outgoing.send(Outgoing::Close).await;
    }

    async fn send(&self, envelope: &Envelope) -> Result<(), LanError> {
        if self.is_closed() {
            return Err(LanError::Closed);
        }
        let bytes = serde_json::to_vec(envelope)
            .map_err(|error| LanError::Protocol(format!("encode message: {error}")))?;
        if bytes.len() > self.inner.max_frame_bytes {
            return Err(LanError::TooLarge(self.inner.max_frame_bytes));
        }
        self.inner
            .outgoing
            .send(Outgoing::Frame(bytes))
            .await
            .map_err(|_| LanError::Closed)
    }
}

/// Accepts channels on this device's private addresses.
pub struct LanListener {
    port: u16,
    addresses: Vec<Ipv4Addr>,
    tasks: Vec<JoinHandle<()>>,
}

impl LanListener {
    /// Listens on `addresses`, all on one port, and answers peers with
    /// `handler`. Every address must be a private one (or loopback when the
    /// config allows it).
    pub async fn bind(
        config: ChannelConfig,
        handler: Arc<dyn RequestHandler>,
        addresses: &[Ipv4Addr],
    ) -> Result<Self, LanError> {
        if addresses.is_empty() {
            return Err(LanError::Io(
                "this device has no private network address to listen on".to_string(),
            ));
        }
        if let Some(address) = addresses
            .iter()
            .find(|ip| !is_sharing_address(IpAddr::V4(**ip), config.allow_loopback))
        {
            return Err(LanError::Io(format!(
                "{address} is not a private network address"
            )));
        }
        let listeners = bind_all(addresses, config.listen_port).await?;
        let port = listeners[0]
            .local_addr()
            .map_err(|error| LanError::Io(error.to_string()))?
            .port();
        let handshakes = Arc::new(Semaphore::new(config.max_pending_handshakes));
        let config = Arc::new(config);
        let tasks = listeners
            .into_iter()
            .map(|listener| {
                tokio::spawn(accept_loop(
                    listener,
                    config.clone(),
                    handler.clone(),
                    handshakes.clone(),
                ))
            })
            .collect();
        Ok(Self {
            port,
            addresses: addresses.to_vec(),
            tasks,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn addresses(&self) -> &[Ipv4Addr] {
        &self.addresses
    }

    /// Stops accepting new connections. Established ones stay open.
    pub fn shutdown(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Drop for LanListener {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn bind_all(addresses: &[Ipv4Addr], preferred: u16) -> Result<Vec<TcpListener>, LanError> {
    if preferred != 0 {
        match bind_on(addresses, preferred).await {
            Ok(listeners) => return Ok(listeners),
            Err(error) => log::debug!(
                "LAN sharing port {} unavailable, taking any free port: {}",
                preferred,
                error
            ),
        }
    }
    let mut last_error = None;
    for _ in 0..BIND_ATTEMPTS {
        let first = TcpListener::bind(SocketAddr::new(IpAddr::V4(addresses[0]), 0))
            .await
            .map_err(|error| LanError::Io(format!("listen on {}: {error}", addresses[0])))?;
        let port = first
            .local_addr()
            .map_err(|error| LanError::Io(error.to_string()))?
            .port();
        let mut listeners = vec![first];
        let mut failed = false;
        for address in &addresses[1..] {
            match TcpListener::bind(SocketAddr::new(IpAddr::V4(*address), port)).await {
                Ok(listener) => listeners.push(listener),
                Err(error) => {
                    last_error = Some(LanError::Io(format!("listen on {address}:{port}: {error}")));
                    failed = true;
                    break;
                }
            }
        }
        if !failed {
            return Ok(listeners);
        }
    }
    Err(last_error.unwrap_or_else(|| LanError::Io("could not listen".to_string())))
}

/// Listens on `port` at every address, or on none of them.
async fn bind_on(addresses: &[Ipv4Addr], port: u16) -> Result<Vec<TcpListener>, LanError> {
    let mut listeners = Vec::with_capacity(addresses.len());
    for address in addresses {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(*address), port))
            .await
            .map_err(|error| LanError::Io(format!("listen on {address}:{port}: {error}")))?;
        listeners.push(listener);
    }
    Ok(listeners)
}

async fn accept_loop(
    listener: TcpListener,
    config: Arc<ChannelConfig>,
    handler: Arc<dyn RequestHandler>,
    handshakes: Arc<Semaphore>,
) {
    loop {
        let (stream, address) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                log::warn!("LAN sharing listener stopped accepting: {error}");
                return;
            }
        };
        if !is_sharing_address(address.ip(), config.allow_loopback) {
            log::debug!("Refused a LAN sharing connection from a non-private address");
            continue;
        }
        let Ok(permit) = handshakes.clone().try_acquire_owned() else {
            log::debug!("Refused a LAN sharing connection: too many handshakes in progress");
            continue;
        };
        let config = config.clone();
        let handler = handler.clone();
        tokio::spawn(async move {
            let accepted = tokio::time::timeout(config.handshake_timeout, async {
                let mut ws =
                    tokio_tungstenite::accept_async_with_config(stream, Some(config.websocket()))
                        .await
                        .map_err(|error| LanError::Io(error.to_string()))?;
                let session = handshake(&mut ws, &config, false, None).await?;
                Ok::<_, LanError>((ws, session))
            })
            .await;
            drop(permit);
            match accepted {
                Ok(Ok((ws, (hello, sealer, opener)))) => {
                    let connection = establish(
                        ws,
                        peer_identity(hello, address),
                        sealer,
                        opener,
                        Some(handler.clone()),
                        config.max_frame_bytes,
                    );
                    handler.connected(connection).await;
                }
                Ok(Err(error)) => log::debug!("LAN sharing handshake failed: {error}"),
                Err(_) => log::debug!("LAN sharing handshake timed out"),
            }
        });
    }
}

/// Opens a channel to `address`. When `expected_public_key` is given, the
/// peer must hold exactly that key, as advertised or as granted.
pub async fn connect(
    config: &ChannelConfig,
    address: SocketAddr,
    expected_public_key: Option<&[u8; 32]>,
    handler: Option<Arc<dyn RequestHandler>>,
) -> Result<LanConnection, LanError> {
    if !is_sharing_address(address.ip(), config.allow_loopback) {
        return Err(LanError::Refused(format!(
            "{} is not a private network address",
            address.ip()
        )));
    }
    let stream = tokio::time::timeout(config.handshake_timeout, TcpStream::connect(address))
        .await
        .map_err(|_| LanError::Unreachable(format!("connect to {address}: timed out")))?
        .map_err(|error| LanError::Unreachable(format!("connect to {address}: {error}")))?;
    let (ws, (hello, sealer, opener)) = tokio::time::timeout(config.handshake_timeout, async {
        let (mut ws, _) = tokio_tungstenite::client_async_with_config(
            format!("ws://{address}/lan-share"),
            stream,
            Some(config.websocket()),
        )
        .await
        .map_err(|error| opening_error(address, error))?;
        let session = handshake(&mut ws, config, true, expected_public_key).await?;
        Ok::<_, LanError>((ws, session))
    })
    .await
    .map_err(|_| LanError::Timeout)??;
    Ok(establish(
        ws,
        peer_identity(hello, address),
        sealer,
        opener,
        handler,
        config.max_frame_bytes,
    ))
}

/// Why the WebSocket opening failed. A peer that closes without a word is
/// treated as unreachable: a proxy or VPN that takes over the route accepts
/// every connection itself and closes it when nothing answers behind it,
/// while any other program at the address answers something.
fn opening_error(address: SocketAddr, error: tungstenite::Error) -> LanError {
    use std::io::ErrorKind;
    use tungstenite::Error;
    let silent = match &error {
        Error::ConnectionClosed | Error::AlreadyClosed => true,
        Error::Io(io) => matches!(
            io.kind(),
            ErrorKind::UnexpectedEof
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::BrokenPipe
        ),
        _ => false,
    };
    if silent {
        LanError::Unreachable(format!(
            "{address} closed the connection without answering: {error}"
        ))
    } else {
        LanError::Io(error.to_string())
    }
}

type Ws = WebSocketStream<TcpStream>;

async fn handshake(
    ws: &mut Ws,
    config: &ChannelConfig,
    initiator: bool,
    expected_public_key: Option<&[u8; 32]>,
) -> Result<(CheckedHello, FrameSealer, FrameOpener), LanError> {
    let local = LocalHandshake::new();
    let hello = local.hello(&config.identity, PROTOCOL_VERSION, &config.capabilities);
    let hello_text = serde_json::to_string(&hello)
        .map_err(|error| LanError::Protocol(format!("encode hello: {error}")))?;
    if initiator {
        send(ws, Message::text(hello_text.clone())).await?;
    }
    let (peer, peer_text) = match next_message(ws).await? {
        Message::Text(text) => (
            serde_json::from_str::<Hello>(text.as_str())
                .map_err(|_| LanError::Protocol("the hello is malformed".to_string()))?
                .check()
                .map_err(LanError::Protocol)?,
            text.as_str().to_string(),
        ),
        _ => return Err(LanError::Protocol("expected a hello".to_string())),
    };
    if peer.protocol != PROTOCOL_VERSION {
        close(ws, CLOSE_PROTOCOL_VERSION, &PROTOCOL_VERSION.to_string()).await;
        return Err(LanError::ProtocolVersion(peer.protocol));
    }
    if let Some(expected) = expected_public_key {
        if &peer.public_key != expected {
            close(ws, CLOSE_KEY_MISMATCH, "").await;
            return Err(LanError::PeerKeyMismatch);
        }
    }
    let missing: Vec<String> = config
        .required_capabilities
        .iter()
        .filter(|capability| !peer.capabilities.contains(capability))
        .cloned()
        .collect();
    if !missing.is_empty() {
        close(ws, CLOSE_NEEDS_UPDATE, &missing.join(",")).await;
        return Err(LanError::PeerNeedsUpdate { missing });
    }
    if !initiator {
        send(ws, Message::text(hello_text.clone())).await?;
    }
    let keys = local
        .finish(&config.identity, &peer, initiator, &hello_text, &peer_text)
        .map_err(LanError::Protocol)?;
    let (mut sealer, mut opener) = (keys.sender, keys.receiver);
    let ready = serde_json::to_vec(&Envelope::Ready).expect("ready encodes");
    send(
        ws,
        Message::binary(sealer.seal(&ready).map_err(LanError::Protocol)?),
    )
    .await?;
    match next_message(ws).await? {
        Message::Binary(frame) => {
            let plain = opener.open(&frame).map_err(|_| LanError::PeerKeyMismatch)?;
            match serde_json::from_slice::<Envelope>(&plain) {
                Ok(Envelope::Ready) => {}
                _ => return Err(LanError::Protocol("expected the ready frame".to_string())),
            }
        }
        _ => return Err(LanError::Protocol("expected the ready frame".to_string())),
    }
    Ok((peer, sealer, opener))
}

async fn send(ws: &mut Ws, message: Message) -> Result<(), LanError> {
    ws.send(message)
        .await
        .map_err(|error| LanError::Io(error.to_string()))
}

async fn close(ws: &mut Ws, code: u16, reason: &str) {
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::from(code),
            reason: reason.to_string().into(),
        })))
        .await;
}

/// The next data message during the handshake. A close from the peer is
/// translated into the reason it gave.
async fn next_message(ws: &mut Ws) -> Result<Message, LanError> {
    loop {
        let message = ws
            .next()
            .await
            .ok_or(LanError::Closed)?
            .map_err(|error| LanError::Io(error.to_string()))?;
        match message {
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(Some(frame)) => {
                let reason = frame.reason.as_str();
                return Err(match u16::from(frame.code) {
                    CLOSE_NEEDS_UPDATE => LanError::NeedsUpdate {
                        missing: reason
                            .split(',')
                            .filter(|item| !item.is_empty())
                            .map(str::to_string)
                            .collect(),
                    },
                    CLOSE_PROTOCOL_VERSION => {
                        LanError::ProtocolVersion(reason.parse().unwrap_or_default())
                    }
                    CLOSE_KEY_MISMATCH => {
                        LanError::Refused("the other device rejected this device's key".to_string())
                    }
                    _ => LanError::Closed,
                });
            }
            Message::Close(None) => return Err(LanError::Closed),
            other => return Ok(other),
        }
    }
}

fn peer_identity(hello: CheckedHello, address: SocketAddr) -> PeerIdentity {
    PeerIdentity {
        fingerprint: fingerprint_of(&hello.public_key),
        device_id: hello.device_id,
        device_name: hello.device_name,
        display_name: hello.display_name,
        public_key: hello.public_key,
        capabilities: hello.capabilities,
        address,
    }
}

fn establish(
    ws: Ws,
    peer: PeerIdentity,
    mut sealer: FrameSealer,
    mut opener: FrameOpener,
    handler: Option<Arc<dyn RequestHandler>>,
    max_frame_bytes: usize,
) -> LanConnection {
    let (mut sink, mut stream) = ws.split();
    let (outgoing, mut outgoing_rx) = mpsc::channel::<Outgoing>(OUTGOING_QUEUE);
    let (push_tx, push_rx) = mpsc::channel(PUSH_QUEUE);
    let (closed_tx, closed_rx) = watch::channel(false);
    let connection = LanConnection {
        inner: Arc::new(Inner {
            peer,
            outgoing: outgoing.clone(),
            pending: std::sync::Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            closed: closed_rx,
            pushes: std::sync::Mutex::new(Some(push_rx)),
            max_frame_bytes,
        }),
    };

    // Writer: seals frames in order. It ends on a close request, a send
    // failure, or when the reader has ended.
    let writer_closed = connection.inner.closed.clone();
    tokio::spawn(async move {
        let mut writer_closed = writer_closed;
        loop {
            let next = tokio::select! {
                next = outgoing_rx.recv() => next,
                _ = writer_closed.wait_for(|closed| *closed) => None,
            };
            match next {
                Some(Outgoing::Frame(plain)) => {
                    let Ok(frame) = sealer.seal(&plain) else {
                        break;
                    };
                    if sink.send(Message::binary(frame)).await.is_err() {
                        break;
                    }
                }
                Some(Outgoing::Close) | None => break,
            }
        }
        let _ = sink.send(Message::Close(None)).await;
        let _ = sink.close().await;
    });

    // Reader: opens frames, answers requests, routes responses and pushes.
    let reader = connection.clone();
    tokio::spawn(async move {
        let inflight = Arc::new(Semaphore::new(MAX_INFLIGHT_REQUESTS));
        while let Some(message) = stream.next().await {
            let frame = match message {
                Ok(Message::Binary(frame)) => frame,
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {
                    log::debug!("LAN sharing peer sent a non-binary frame; closing");
                    break;
                }
            };
            let envelope = match opener
                .open(&frame)
                .ok()
                .and_then(|plain| serde_json::from_slice::<Envelope>(&plain).ok())
            {
                Some(envelope) => envelope,
                None => {
                    log::debug!("LAN sharing peer sent an unreadable frame; closing");
                    let _ = reader.inner.outgoing.try_send(Outgoing::Close);
                    break;
                }
            };
            match envelope {
                Envelope::Request { id, method, body } => {
                    let Ok(permit) = inflight.clone().try_acquire_owned() else {
                        let _ = reader
                            .send(&Envelope::Response {
                                id,
                                body: Value::Null,
                                error: Some("the device is busy; try again".to_string()),
                            })
                            .await;
                        continue;
                    };
                    let handler = handler.clone();
                    let connection = reader.clone();
                    tokio::spawn(async move {
                        let result = match handler {
                            Some(handler) => handler.handle(connection.peer(), &method, body).await,
                            None => Err("this device does not accept requests here".to_string()),
                        };
                        let response = match result {
                            Ok(body) => Envelope::Response {
                                id,
                                body,
                                error: None,
                            },
                            Err(error) => Envelope::Response {
                                id,
                                body: Value::Null,
                                error: Some(error),
                            },
                        };
                        if let Err(LanError::TooLarge(limit)) = connection.send(&response).await {
                            let _ = connection
                                .send(&Envelope::Response {
                                    id,
                                    body: Value::Null,
                                    error: Some(format!(
                                        "the answer is larger than the {limit}-byte limit"
                                    )),
                                })
                                .await;
                        }
                        drop(permit);
                    });
                }
                Envelope::Response { id, body, error } => {
                    let waiter = reader
                        .inner
                        .pending
                        .lock()
                        .expect("pending lock")
                        .remove(&id);
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(match error {
                            Some(error) => Err(LanError::Remote(error)),
                            None => Ok(body),
                        });
                    }
                }
                Envelope::Push { topic, body } => {
                    if push_tx.send(LanPush { topic, body }).await.is_err() {
                        log::debug!("Dropped a LAN sharing push nobody reads");
                    }
                }
                Envelope::Ready => {
                    log::debug!("LAN sharing peer repeated the ready frame; closing");
                    break;
                }
            }
        }
        let _ = closed_tx.send(true);
        let waiters: Vec<_> = reader
            .inner
            .pending
            .lock()
            .expect("pending lock")
            .drain()
            .map(|(_, waiter)| waiter)
            .collect();
        for waiter in waiters {
            let _ = waiter.send(Err(LanError::Closed));
        }
    });

    connection
}
