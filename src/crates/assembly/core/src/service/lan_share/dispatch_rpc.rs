//! Dispatch to a shared assistant over the LAN share channel.
//!
//! The Dispatch controller reaches another device through
//! [`DeviceDispatchRpc`]; for an assistant shared on the local network the
//! device is its owner, and the call goes over the channel pinned to the key
//! this device recorded when access was granted. The owner binds each
//! command to this device's key before its executor sees it.

use async_trait::async_trait;
use serde_json::Value;

use super::host::LanShareHost;
use crate::service::dispatch::DeviceDispatchRpc;

/// [`DeviceDispatchRpc`] over the LAN share host. Errors are
/// [`ShareDispatchError`](super::host::ShareDispatchError) inside
/// `anyhow`, so callers can tell offline, withdrawn and outdated owners apart.
#[derive(Clone)]
pub struct LanDispatchRpc {
    host: LanShareHost,
}

impl LanDispatchRpc {
    pub fn new(host: LanShareHost) -> Self {
        Self { host }
    }
}

#[async_trait]
impl DeviceDispatchRpc for LanDispatchRpc {
    async fn invoke(&self, device_id: &str, command: &str, args: Value) -> anyhow::Result<Value> {
        Ok(self.host.dispatch(device_id, command, args).await?)
    }
}
