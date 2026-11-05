//! Sharing assistants on the local network (F002 RFC v6).
//!
//! The owner's share store and the per-device tenant workspaces the shared
//! assistants run in. Who may request, confirm, revoke, or remove is decided
//! here; `openbitfun-services-integrations::lan_share` only provides the
//! sharing identity, discovery, and the encrypted channel.

pub mod added_devices;
#[cfg(feature = "lan-share")]
pub mod browse;
#[cfg(all(
    feature = "lan-share",
    feature = "dispatch-store",
    feature = "agent-runtime",
    feature = "ssh-remote"
))]
pub mod client;
#[cfg(all(
    feature = "lan-share",
    feature = "dispatch-store",
    feature = "agent-runtime",
    feature = "ssh-remote"
))]
pub mod dispatch_rpc;
#[cfg(feature = "lan-share")]
pub mod host;
pub mod protocol;
#[cfg(all(
    feature = "lan-share",
    feature = "dispatch-store",
    feature = "agent-runtime",
    feature = "ssh-remote"
))]
pub mod remote_agents;
pub mod requests;
pub mod shared_assistants;
pub mod shared_with_me;
pub mod tenants;

#[cfg(test)]
mod tests;

pub use added_devices::{AddedDevice, AddedDeviceStore, AddedDevicesError};
#[cfg(feature = "lan-share")]
pub use browse::{DeviceStatus, DiscoveredAssistant, DiscoveredDevice};
#[cfg(feature = "lan-share")]
pub use host::{
    AddressOutcome, AddressProblem, AddressedDevice, HostStatus, LanShareHost, LanShareOptions,
    RequestAnswer, ShareDispatch, ShareDispatchError, ShareRequestView, SharedWithMeNow,
    EVENT_DISCOVERED, EVENT_JOB_UPDATED, EVENT_REQUESTS, EVENT_SHARED, EVENT_SHARED_WITH_ME,
};
pub use shared_assistants::{
    is_share_device_id, share_device_id, BlockedDevice, ShareConfirmations, ShareGrant, SharePeer,
    ShareRefusal, ShareSettings, ShareStoreError, SharedAssistant, SharedAssistantStore,
    DEFAULT_MAX_CONCURRENT_TASKS, DEFAULT_SESSION_RETENTION_DAYS, MAX_CONCURRENT_TASKS,
    SESSION_RETENTION_DAY_CHOICES,
};
pub use shared_with_me::{
    PendingRemoval, SharedWithMe, SharedWithMeError, SharedWithMeState, SharedWithMeStore,
};
#[cfg(feature = "local-storage")]
pub use tenants::prune_tenants;
pub use tenants::{
    delete_tenant, ensure_tenant, is_tenant_path, list_tenants, local_share_device_id, tenant_dir,
    tenant_user, tenants_root, Tenant, TenantUser,
};
