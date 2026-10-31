//! Desktop host for delegating subagent turns to agents other devices of the
//! signed-in account opened to this one, and to agents paired devices of
//! other accounts granted to it.
//!
//! The kernel owns delegation semantics in `service::dispatch`; this adapter
//! only supplies the account's device list, the stored connections, and the
//! encrypted relay RPC of each.

use std::sync::Arc;

use async_trait::async_trait;
use openbitfun_core::service::dispatch::{
    AccountDevice, AccountDeviceDirectory, ConnectionDispatchRpc, ConnectionPeer,
    DeviceDispatchRpc, RemoteAgentDirectory,
};
use openbitfun_core::service::remote_connect::connection_rpc::{
    usable_connection_peers, ConnectionRpc,
};
use openbitfun_core::service::remote_connect::connections::ConnectionStore;
use openbitfun_core::service::remote_connect::AccountClient;
use serde_json::Value;

struct SignedInAccountDevices;

#[async_trait]
impl AccountDeviceDirectory for SignedInAccountDevices {
    async fn other_devices(&self) -> anyhow::Result<Vec<AccountDevice>> {
        let current_device_id = super::remote_connect_api::remote_connect_get_device_info()
            .await
            .map_err(anyhow::Error::msg)?
            .device_id;
        let devices = super::remote_connect_api::account_list_devices()
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(devices
            .into_iter()
            .filter(|device| device.device_id != current_device_id)
            .map(|device| AccountDevice {
                device_id: device.device_id,
                device_name: device.device_name,
                online: device.online,
            })
            .collect())
    }
}

/// The stored connections of the signed-in device, called over its own
/// routing socket.
#[derive(Default)]
struct SignedInConnections {
    /// The Relay's capabilities, kept per Relay URL so observing a job does
    /// not ask for them on every page.
    capabilities: tokio::sync::Mutex<Option<(String, Vec<String>)>>,
}

impl SignedInConnections {
    async fn relay_capabilities(&self, relay_url: &str) -> Vec<String> {
        let mut cached = self.capabilities.lock().await;
        if let Some((url, capabilities)) = cached.as_ref() {
            if url == relay_url && !capabilities.is_empty() {
                return capabilities.clone();
            }
        }
        let capabilities = AccountClient::new()
            .relay_capabilities(relay_url)
            .await
            .unwrap_or_default();
        *cached = Some((relay_url.to_string(), capabilities.clone()));
        capabilities
    }
}

#[async_trait]
impl ConnectionDispatchRpc for SignedInConnections {
    async fn usable_connections(&self) -> anyhow::Result<Vec<ConnectionPeer>> {
        let (_, context) = super::remote_connect_api::current_connection_context()
            .await
            .map_err(anyhow::Error::msg)?;
        usable_connection_peers(&ConnectionStore::open_default()?, &context)
    }

    async fn invoke_connection(
        &self,
        connection_id: &str,
        command: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let (relay, context) = super::remote_connect_api::current_connection_context()
            .await
            .map_err(anyhow::Error::msg)?;
        let capabilities = self.relay_capabilities(&context.relay_url).await;
        ConnectionRpc::new(
            relay,
            context,
            ConnectionStore::open_default()?,
            capabilities,
        )
        .invoke_connection(connection_id, command, args)
        .await
    }

    fn gone_reason(
        &self,
        error: &anyhow::Error,
    ) -> Option<openbitfun_core::service::dispatch::GoneConnection> {
        openbitfun_core::service::remote_connect::connection_rpc::gone_connection(error)
    }
}

/// The port the coordinator uses for `remote:` agents on this host.
pub(crate) fn remote_agent_port() -> Arc<dyn openbitfun_runtime_ports::RemoteAgentDelegationPort> {
    let rpc: Arc<dyn DeviceDispatchRpc> = Arc::new(super::dispatch_api::AccountDeviceDispatchRpc);
    Arc::new(
        RemoteAgentDirectory::new(rpc, Arc::new(SignedInAccountDevices))
            .with_connections(Arc::new(SignedInConnections::default())),
    )
}

#[cfg(test)]
mod simulation_tests;
