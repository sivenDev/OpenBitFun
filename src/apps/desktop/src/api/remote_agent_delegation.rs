//! Desktop host for delegating subagent turns to agents other devices of the
//! signed-in account opened to this one.
//!
//! The kernel owns delegation semantics in `service::dispatch`; this adapter
//! only supplies the account's device list and the encrypted relay RPC.

use std::sync::Arc;

use async_trait::async_trait;
use openbitfun_core::service::dispatch::{
    AccountDevice, AccountDeviceDirectory, DeviceDispatchRpc, RemoteAgentDirectory,
};

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

/// The port the coordinator uses for `remote:` agents on this host.
pub(crate) fn remote_agent_port() -> Arc<dyn openbitfun_runtime_ports::RemoteAgentDelegationPort> {
    let rpc: Arc<dyn DeviceDispatchRpc> = Arc::new(super::dispatch_api::AccountDeviceDispatchRpc);
    Arc::new(RemoteAgentDirectory::new(
        rpc,
        Arc::new(SignedInAccountDevices),
    ))
}
