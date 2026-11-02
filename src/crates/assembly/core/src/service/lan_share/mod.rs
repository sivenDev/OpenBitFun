//! Sharing assistants on the local network (F002 RFC v6).
//!
//! The owner's share store and the per-device tenant workspaces the shared
//! assistants run in. Who may request, confirm, revoke, or remove is decided
//! here; `openbitfun-services-integrations::lan_share` only provides the
//! sharing identity, discovery, and the encrypted channel.

#[cfg(feature = "lan-share")]
pub mod browse;
#[cfg(feature = "lan-share")]
pub mod host;
pub mod protocol;
pub mod requests;
pub mod shared_assistants;
pub mod shared_with_me;
pub mod tenants;

#[cfg(test)]
mod tests;

#[cfg(feature = "lan-share")]
pub use browse::{DeviceStatus, DiscoveredAssistant, DiscoveredDevice};
#[cfg(feature = "lan-share")]
pub use host::{
    HostStatus, LanShareHost, LanShareOptions, RequestAnswer, ShareRequestView, EVENT_DISCOVERED,
    EVENT_REQUESTS, EVENT_SHARED, EVENT_SHARED_WITH_ME,
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
    delete_tenant, ensure_tenant, is_tenant_path, list_tenants, tenant_dir, tenants_root, Tenant,
    TenantUser,
};
