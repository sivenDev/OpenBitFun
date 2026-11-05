//! The owners of assistants shared with this device, reached without the
//! share host.
//!
//! A process that runs a task another device delegated here (the Dispatch
//! worker) has no share host: the app owns listening, Bonjour and the
//! background channels. When the owner fully trusts that share, the task may
//! still call assistants shared with this device, so the worker opens its own
//! channel to their owners as this device. It reads this device's sharing
//! identity without ever creating one, finds each owner where the app last
//! reached it, and neither listens nor advertises. An owner it cannot reach is
//! reported offline; the app's own channels are untouched.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use log::debug;
use openbitfun_services_integrations::lan_share::{
    connect, ChannelConfig, LanConnection, LanError, ShareIdentity,
};
use serde_json::Value;

use super::added_devices::AddedDeviceStore;
use super::host::{
    available_from, call_dispatch, key_bytes, unreachable_owner, ShareDispatchError,
    SharedWithMeNow, IDENTITY_FILE, LAN_SHARE_DIR,
};
use super::protocol::ASSISTANT_SHARE_CAPABILITY;
use super::remote_agents::SharedWithMePresence;
use super::shared_with_me::SharedWithMeStore;
use crate::infrastructure::PathManager;
use crate::service::dispatch::DeviceDispatchRpc;

/// Channels to the owners of assistants shared with this device, opened by
/// a process that is not the share host.
pub struct LanShareClient {
    identity: Arc<ShareIdentity>,
    shared_with_me: SharedWithMeStore,
    added_devices: AddedDeviceStore,
    allow_loopback: bool,
    owners: tokio::sync::Mutex<HashMap<String, LanConnection>>,
}

impl LanShareClient {
    /// This device's client, when the app has given it a sharing identity.
    pub fn for_product_home(path_manager: &PathManager) -> Result<Self, String> {
        let dir = path_manager.product_home_dir().join(LAN_SHARE_DIR);
        Self::open(
            &dir.join(IDENTITY_FILE),
            SharedWithMeStore::for_product_home(path_manager),
            AddedDeviceStore::for_product_home(path_manager),
        )
    }

    pub fn open(
        identity_path: &Path,
        shared_with_me: SharedWithMeStore,
        added_devices: AddedDeviceStore,
    ) -> Result<Self, String> {
        let identity = ShareIdentity::load(identity_path).map_err(|error| format!("{error:#}"))?;
        Ok(Self {
            identity: Arc::new(identity),
            shared_with_me,
            added_devices,
            allow_loopback: false,
            owners: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Accept 127.0.0.1, for single-machine tests.
    pub fn allow_loopback(mut self) -> Self {
        self.allow_loopback = true;
        self
    }

    /// Sends a Dispatch target command to the device `device_id`, which
    /// shares an assistant with this one.
    pub async fn dispatch(
        &self,
        device_id: &str,
        command: &str,
        args: Value,
    ) -> Result<Value, ShareDispatchError> {
        let entries = self
            .shared_with_me
            .assistants()
            .await
            .map_err(|error| ShareDispatchError::Refused(error.to_string()))?;
        let entry = available_from(&entries, device_id)?;
        let owner_key = entry.owner.public_key.clone();
        let public_key = key_bytes(&owner_key).ok_or(ShareDispatchError::NotShared)?;
        let mut addresses: Vec<SocketAddr> = entry
            .last_address
            .as_deref()
            .and_then(|address| address.parse().ok())
            .into_iter()
            .collect();
        if let Ok(Some(added)) = self.added_devices.device_with_key(&owner_key).await {
            if let Ok(address) = added.address.parse() {
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
            }
        }
        let connection = self
            .connection(&owner_key, &addresses, &public_key)
            .await
            .map_err(unreachable_owner)?;
        call_dispatch(&connection, command, args).await
    }

    async fn connection(
        &self,
        owner_key: &str,
        addresses: &[SocketAddr],
        public_key: &[u8; 32],
    ) -> Result<LanConnection, LanError> {
        let mut owners = self.owners.lock().await;
        if let Some(open) = owners.get(owner_key).filter(|open| !open.is_closed()) {
            return Ok(open.clone());
        }
        let mut config = ChannelConfig::new(self.identity.clone());
        config.capabilities = vec![ASSISTANT_SHARE_CAPABILITY.to_string()];
        config.required_capabilities = config.capabilities.clone();
        config.allow_loopback = self.allow_loopback;
        let mut failure =
            LanError::Io("the device has no address on the local network".to_string());
        for address in addresses {
            match connect(&config, *address, Some(public_key), None).await {
                Ok(connection) => {
                    // The owner's news is for the app; calls are polled.
                    if let Some(mut pushes) = connection.take_pushes() {
                        tokio::spawn(async move { while pushes.recv().await.is_some() {} });
                    }
                    owners.insert(owner_key.to_string(), connection.clone());
                    return Ok(connection);
                }
                Err(error) => {
                    debug!("Owner not reached: address={}, error={}", address, error);
                    failure = error;
                }
            }
        }
        Err(failure)
    }
}

#[async_trait]
impl SharedWithMePresence for LanShareClient {
    /// Whether an owner can be reached shows only when a call is tried, so
    /// each assistant is listed as reachable and a call reports otherwise.
    async fn shared_with_me_now(&self) -> Result<Vec<SharedWithMeNow>, String> {
        Ok(self
            .shared_with_me
            .assistants()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|assistant| SharedWithMeNow {
                assistant,
                online: true,
            })
            .collect())
    }

    fn local_device_id(&self) -> Option<String> {
        Some(self.identity.device_id().to_string())
    }
}

#[async_trait]
impl DeviceDispatchRpc for LanShareClient {
    async fn invoke(&self, device_id: &str, command: &str, args: Value) -> anyhow::Result<Value> {
        self.dispatch(device_id, command, args)
            .await
            .map_err(super::dispatch_rpc::problem)
    }
}
