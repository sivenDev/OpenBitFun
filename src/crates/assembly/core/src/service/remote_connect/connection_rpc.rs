//! Device RPC over a cross-account connection.
//!
//! The requester side implements `DeviceDispatchRpc` with a connection id as
//! the target: it encrypts each request with the connection key and sends it
//! through the Relay's cross-account route. The target side checks a call
//! against its own pinned record (connection, source account and device,
//! routed command) before a handler sees it. Until a device grants items to a
//! connection (F002-06), its handler refuses every call.

use std::sync::Arc;

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use openbitfun_services_integrations::remote_connect::account::AccountSession;
use openbitfun_services_integrations::remote_connect::connections::{
    self, ConnectionStore, Direction, ReplayGuard, RouteRefusal,
    RELAY_CAPABILITY_CROSS_ACCOUNT_ROUTING,
};
use openbitfun_services_integrations::remote_connect::relay_client::RelayClient;

/// The signed-in device that owns connections: session, Relay and device id.
#[derive(Debug, Clone)]
pub struct ConnectionContext {
    pub session: AccountSession,
    pub relay_url: String,
    pub local_device_id: String,
}

/// The device id the Relay knows this session by: the one persisted with the
/// session at sign-in or `AuthOk`, else this machine's identity.
pub fn authenticated_device_id(session: &AccountSession) -> Result<String> {
    use openbitfun_services_integrations::remote_connect::{session_store, DeviceIdentity};
    if let Ok(Some(loaded)) = session_store::load_session_detailed() {
        if loaded.token == session.token {
            if let Some(device_id) = loaded.device_id {
                return Ok(device_id);
            }
        }
    }
    Ok(DeviceIdentity::from_current_machine()?.device_id)
}

/// The routing connection a request travels over.
#[async_trait]
pub trait ConnectionTransport: Send + Sync {
    async fn request_connection(
        &self,
        connection_id: &str,
        command: &str,
        encrypted_data: &str,
        nonce: &str,
    ) -> Result<Value>;
}

#[async_trait]
impl ConnectionTransport for RelayClient {
    async fn request_connection(
        &self,
        connection_id: &str,
        command: &str,
        encrypted_data: &str,
        nonce: &str,
    ) -> Result<Value> {
        RelayClient::request_connection(self, connection_id, command, encrypted_data, nonce).await
    }
}

fn route_refusal(reason: &str, message: &str) -> anyhow::Error {
    RouteRefusal {
        reason: reason.to_owned(),
        message: message.to_owned(),
    }
    .into()
}

/// Calls paired devices of other accounts. Must use the device's own routing
/// connection: the Relay only routes connection calls from device sockets.
pub struct ConnectionRpc {
    transport: Arc<dyn ConnectionTransport>,
    context: ConnectionContext,
    store: ConnectionStore,
    relay_capabilities: Vec<String>,
}

impl ConnectionRpc {
    pub fn new(
        transport: Arc<dyn ConnectionTransport>,
        context: ConnectionContext,
        store: ConnectionStore,
        relay_capabilities: Vec<String>,
    ) -> Self {
        Self {
            transport,
            context,
            store,
            relay_capabilities,
        }
    }

    pub fn relay_supports_connections(capabilities: &[String]) -> bool {
        capabilities
            .iter()
            .any(|capability| capability == RELAY_CAPABILITY_CROSS_ACCOUNT_ROUTING)
    }

    pub async fn invoke_connection(
        &self,
        connection_id: &str,
        command: &str,
        args: Value,
    ) -> Result<Value> {
        if !Self::relay_supports_connections(&self.relay_capabilities) {
            return Err(route_refusal(
                "relay_unsupported",
                "the relay does not support cross-account connections",
            ));
        }
        if !connections::is_connection_command(command) {
            return Err(route_refusal(
                "command_not_routable",
                "command is not routable across accounts",
            ));
        }
        let record = self.store.usable(
            connection_id,
            &self.context.relay_url,
            &self.context.local_device_id,
        )?;
        let key = record.connection_key(&self.context.session.master_key)?;
        let sealed = connections::seal_request(
            &key,
            Direction {
                connection_id,
                from_device_id: &self.context.local_device_id,
                to_device_id: &record.peer_device_id,
            },
            command,
            &args,
        )?;
        let answer = match self
            .transport
            .request_connection(
                connection_id,
                command,
                &sealed.encrypted_data,
                &sealed.nonce,
            )
            .await
        {
            Ok(answer) => answer,
            Err(error) => {
                if let Some(reason) = connections::refusal_reason(&error) {
                    log::info!("Connection call refused by relay: connection={connection_id} reason={reason}");
                    if let Err(store_error) = self.store.note_refusal(connection_id, reason) {
                        log::warn!("Failed to record connection refusal: {store_error}");
                    }
                }
                return Err(error);
            }
        };
        if let Some(reason) = answer["refused"].as_str() {
            log::info!(
                "Connection call refused by the other device: connection={connection_id} reason={reason}"
            );
            return Err(route_refusal(
                reason,
                "the other device does not accept this connection",
            ));
        }
        let (Some(data), Some(nonce)) =
            (answer["encrypted_data"].as_str(), answer["nonce"].as_str())
        else {
            bail!("connection answer is malformed");
        };
        connections::open_response(
            &key,
            Direction {
                connection_id,
                from_device_id: &record.peer_device_id,
                to_device_id: &self.context.local_device_id,
            },
            &sealed.request_id,
            data,
            nonce,
        )
    }
}

#[cfg(all(feature = "dispatch-store", feature = "ssh-remote"))]
#[async_trait]
impl crate::service::dispatch::DeviceDispatchRpc for ConnectionRpc {
    /// `device_id` is the connection id for this transport.
    async fn invoke(&self, device_id: &str, command: &str, args: Value) -> Result<Value> {
        self.invoke_connection(device_id, command, args).await
    }
}

// ── Target side ────────────────────────────────────────────────────────

/// Who is calling: the Relay's authenticated source, confirmed against this
/// device's pinned record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionCaller {
    pub connection_id: String,
    pub peer_user_id: String,
    pub peer_display_name: String,
    pub peer_device_id: String,
}

#[async_trait]
pub trait ConnectionRequestHandler: Send + Sync {
    async fn handle(
        &self,
        caller: &ConnectionCaller,
        command: &str,
        args: Value,
    ) -> std::result::Result<Value, String>;
}

pub const CROSS_ACCOUNT_NOT_ENABLED: &str =
    "cross-account delegation is not enabled on this device";

/// The handler of a device that has granted nothing to any connection.
pub struct RefuseConnectionRequests;

#[async_trait]
impl ConnectionRequestHandler for RefuseConnectionRequests {
    async fn handle(
        &self,
        _caller: &ConnectionCaller,
        _command: &str,
        _args: Value,
    ) -> std::result::Result<Value, String> {
        Err(CROSS_ACCOUNT_NOT_ENABLED.to_owned())
    }
}

/// A call as the Relay delivered it.
#[derive(Debug, Clone)]
pub struct IncomingConnectionRequest {
    pub connection_id: String,
    pub command: String,
    pub source_account_id: String,
    pub source_device_id: String,
    pub encrypted_data: String,
    pub nonce: String,
}

/// Request ids this process has accepted; see `connections::ReplayGuard`.
fn replay_guard() -> &'static ReplayGuard {
    static GUARD: std::sync::OnceLock<ReplayGuard> = std::sync::OnceLock::new();
    GUARD.get_or_init(ReplayGuard::default)
}

fn refused(reason: &str) -> Value {
    json!({ "refused": reason })
}

/// The answer to send back. It is encrypted with the connection key, or a
/// bare refusal reason when this device has no usable key for the call.
pub async fn answer_connection_request(
    context: &ConnectionContext,
    store: &ConnectionStore,
    request: &IncomingConnectionRequest,
    handler: &dyn ConnectionRequestHandler,
) -> Value {
    let record = match store.usable(
        &request.connection_id,
        &context.relay_url,
        &context.local_device_id,
    ) {
        Ok(record) => record,
        Err(error) => {
            let reason = connections::refusal_reason(&error).unwrap_or("connection_unusable");
            log::info!(
                "Connection request refused: connection={} reason={reason}",
                request.connection_id
            );
            return refused(reason);
        }
    };
    if record.peer_device_id != request.source_device_id
        || record.peer_user_id != request.source_account_id
    {
        log::warn!(
            "Connection request refused: connection={} reason=connection_mismatch",
            request.connection_id
        );
        return refused("connection_mismatch");
    }
    let key = match record.connection_key(&context.session.master_key) {
        Ok(key) => key,
        Err(error) => {
            log::warn!("Connection key unavailable: {error}");
            return refused("connection_unusable");
        }
    };
    let opened = match connections::open_request(
        &key,
        Direction {
            connection_id: &record.connection_id,
            from_device_id: &record.peer_device_id,
            to_device_id: &context.local_device_id,
        },
        &request.command,
        &request.encrypted_data,
        &request.nonce,
    ) {
        Ok(opened) => opened,
        Err(error) => {
            // Without an authentic request there is no id to answer under.
            log::warn!(
                "Connection request rejected: connection={} error={error}",
                request.connection_id
            );
            return refused("request_rejected");
        }
    };
    let result = match replay_guard().admit(&opened, chrono::Utc::now().timestamp()) {
        Err(error) => {
            log::warn!(
                "Connection request rejected: connection={} error={error}",
                request.connection_id
            );
            Err(error.to_string())
        }
        Ok(()) if connections::is_connection_command(&opened.command) => {
            let caller = ConnectionCaller {
                connection_id: record.connection_id.clone(),
                peer_user_id: record.peer_user_id.clone(),
                peer_display_name: record.peer_display_name.clone(),
                peer_device_id: record.peer_device_id.clone(),
            };
            log::info!(
                "Connection request: connection={} command={}",
                request.connection_id,
                opened.command
            );
            handler
                .handle(&caller, &opened.command, opened.args.clone())
                .await
        }
        Ok(()) => Err("command is not routable across accounts".to_owned()),
    };
    let reply = Direction {
        connection_id: &record.connection_id,
        from_device_id: &context.local_device_id,
        to_device_id: &record.peer_device_id,
    };
    match connections::seal_response(&key, reply, &opened.request_id, result) {
        Ok((data, nonce)) => json!({ "encrypted_data": data, "nonce": nonce }),
        Err(error) => {
            log::warn!("Failed to encrypt connection answer: {error}");
            refused("connection_unusable")
        }
    }
}

/// Answer one delivered call on `relay` with the default (refusing) handler
/// or the one given. Errors are logged; the caller sees a refusal or timeout.
pub async fn serve_connection_request(
    relay: &RelayClient,
    context: &ConnectionContext,
    request: IncomingConnectionRequest,
    correlation_id: &str,
    handler: &dyn ConnectionRequestHandler,
) {
    let store = match ConnectionStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            log::warn!("Connection store unavailable: {error}");
            let _ = relay
                .respond_connection(
                    &request.source_device_id,
                    correlation_id,
                    refused("connection_unusable"),
                )
                .await;
            return;
        }
    };
    let answer = answer_connection_request(context, &store, &request, handler).await;
    if let Err(error) = relay
        .respond_connection(&request.source_device_id, correlation_id, answer)
        .await
    {
        log::warn!("Failed to answer connection request: {error}");
    }
}

impl IncomingConnectionRequest {
    /// Build from a `RelayEvent::ConnectionRequest`; other events give `None`.
    pub fn from_event(
        event: &openbitfun_services_integrations::remote_connect::RelayEvent,
    ) -> Option<(Self, String)> {
        match event {
            openbitfun_services_integrations::remote_connect::RelayEvent::ConnectionRequest {
                connection_id,
                command,
                source_account_id,
                source_device_id,
                correlation_id,
                encrypted_data,
                nonce,
            } => Some((
                Self {
                    connection_id: connection_id.clone(),
                    command: command.clone(),
                    source_account_id: source_account_id.clone(),
                    source_device_id: source_device_id.clone(),
                    encrypted_data: encrypted_data.clone(),
                    nonce: nonce.clone(),
                },
                correlation_id.clone(),
            )),
            _ => None,
        }
    }
}

/// Fails unless the relay can route between accounts.
pub fn require_connection_support(capabilities: &[String]) -> Result<()> {
    if ConnectionRpc::relay_supports_connections(capabilities) {
        Ok(())
    } else {
        Err(route_refusal(
            "relay_unsupported",
            "the relay does not support cross-account connections",
        ))
    }
}

#[cfg(test)]
mod tests;
