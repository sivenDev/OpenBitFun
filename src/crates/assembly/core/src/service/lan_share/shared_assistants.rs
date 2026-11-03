//! The owner's shared assistants: settings, grants, and blocked devices.
//!
//! The store is device-local (`<product home>/delegation/shared-assistants.json`)
//! and never travels with exported configuration, so a restored backup never
//! shares anything on another device. A file this build cannot read is
//! reported and never overwritten.
//!
//! Grants are keyed by the requesting device's sharing public key. The device
//! id is derived from that key here, never taken from the caller, so a device
//! that shows a known id with another key is a different device.

use std::path::{Path, PathBuf};

use openbitfun_services_core::json_store::{JsonFileStore, JsonFileStoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::infrastructure::PathManager;
use crate::service::delegation::{assistant_agent_type, assistant_directory, DelegationTrust};

const DELEGATION_DIR: &str = "delegation";
const SHARED_ASSISTANTS_FILE: &str = "shared-assistants.json";
const SCHEMA_VERSION: u32 = 1;
const MAX_NAME_CHARS: usize = 64;
/// The longest description other devices see for a shared assistant.
const MAX_DESCRIPTION_CHARS: usize = 200;
const FINGERPRINT_CHARS: usize = 6;

/// How long a shared assistant's sessions are kept after their last turn
/// ends, unless its share says otherwise.
pub const DEFAULT_SESSION_RETENTION_DAYS: u32 = 7;
/// The retention periods an owner can choose, in days.
pub const SESSION_RETENTION_DAY_CHOICES: [u32; 4] = [1, 7, 30, 90];
/// Tasks a shared assistant runs at once for all its users by default.
pub const DEFAULT_MAX_CONCURRENT_TASKS: u32 = 2;
/// Bound on the concurrent tasks an owner can set, so a typo cannot disable it.
pub const MAX_CONCURRENT_TASKS: u32 = 100;

/// What the owner chooses when sharing an assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareSettings {
    pub trust: DelegationTrust,
    pub session_retention_days: u32,
    /// Tasks that run at once for all users together; more wait, queued.
    pub max_concurrent_tasks: u32,
}

impl Default for ShareSettings {
    fn default() -> Self {
        Self {
            trust: DelegationTrust::Normal,
            session_retention_days: DEFAULT_SESSION_RETENTION_DAYS,
            max_concurrent_tasks: DEFAULT_MAX_CONCURRENT_TASKS,
        }
    }
}

impl ShareSettings {
    fn validate(&self) -> Result<(), String> {
        if !SESSION_RETENTION_DAY_CHOICES.contains(&self.session_retention_days) {
            return Err(format!(
                "sessions are kept for one of {SESSION_RETENTION_DAY_CHOICES:?} days"
            ));
        }
        if !(1..=MAX_CONCURRENT_TASKS).contains(&self.max_concurrent_tasks) {
            return Err(format!("concurrent tasks must be 1-{MAX_CONCURRENT_TASKS}"));
        }
        Ok(())
    }
}

/// What the owner explicitly confirmed with a share or a settings change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareConfirmations {
    /// The risk of full trust, required to switch it on.
    #[serde(default)]
    pub full_trust_risk: bool,
    /// That tasks may call external agents with this device's own login or
    /// credentials; recorded on the share once given.
    #[serde(default)]
    pub external_agent_usage: bool,
}

/// A device on the other side of a share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharePeer {
    /// The device's sharing public key, 64 lowercase hex digits.
    pub public_key: String,
    /// Derived from the public key.
    pub device_id: String,
    /// The short code people compare, for display only.
    pub fingerprint: String,
    pub display_name: String,
    pub device_name: String,
}

impl SharePeer {
    /// A peer for `public_key` (64 hex digits), with its device id derived
    /// from the key. Names are trimmed and bounded.
    pub fn new(
        public_key: &str,
        fingerprint: &str,
        display_name: &str,
        device_name: &str,
    ) -> Result<Self, String> {
        let public_key = public_key.trim().to_ascii_lowercase();
        let device_id = share_device_id(&public_key)?;
        let peer = Self {
            public_key,
            device_id,
            fingerprint: fingerprint.trim().to_string(),
            display_name: bounded_name(display_name),
            device_name: bounded_name(device_name),
        };
        peer.validate()?;
        Ok(peer)
    }

    fn validate(&self) -> Result<(), String> {
        if share_device_id(&self.public_key)? != self.device_id {
            return Err("the device id does not belong to the public key".to_string());
        }
        if self.fingerprint.chars().count() != FINGERPRINT_CHARS
            || !self
                .fingerprint
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric())
        {
            return Err("the fingerprint must be 6 letters or digits".to_string());
        }
        if self.display_name.is_empty() || self.device_name.is_empty() {
            return Err("a device needs a display name and a device name".to_string());
        }
        Ok(())
    }
}

/// The device id of a sharing public key given as 64 hex digits: 32 hex
/// digits of its SHA-256, as the LAN layer derives it.
pub fn share_device_id(public_key_hex: &str) -> Result<String, String> {
    let key = hex::decode(public_key_hex.trim())
        .ok()
        .filter(|key| key.len() == 32)
        .ok_or_else(|| "a sharing public key is 64 hex digits".to_string())?;
    Ok(hex::encode(&Sha256::digest(&key)[16..]))
}

/// Whether `value` has the shape of a device id derived from a sharing key.
pub fn is_share_device_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn bounded_name(value: &str) -> String {
    bounded_text(value, MAX_NAME_CHARS)
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(max_chars)
        .collect()
}

/// A device allowed to use a shared assistant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareGrant {
    #[serde(flatten)]
    pub peer: SharePeer,
    /// RFC 3339.
    pub granted_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
}

/// A device whose requests for a shared assistant end without asking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedDevice {
    #[serde(flatten)]
    pub peer: SharePeer,
    pub blocked_at: String,
}

/// One assistant this device shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedAssistant {
    /// The assistant's directory under the assistant workspace base; the id
    /// peers address.
    pub id: String,
    /// The name the owner sees for the assistant, as other devices see it.
    /// Kept current by the share host; `None` until it is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// What the assistant is like, as the owner sees it; other devices see
    /// it when they look the assistant up. Kept current by the share host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub settings: ShareSettings,
    /// The owner confirmed that tasks may call external agents with this
    /// device's own login or credentials.
    #[serde(default)]
    pub external_agent_usage_confirmed: bool,
    pub shared_at: String,
    #[serde(default)]
    pub grants: Vec<ShareGrant>,
    #[serde(default)]
    pub blocked: Vec<BlockedDevice>,
}

impl SharedAssistant {
    /// The agent type the assistant's tasks run as, `assistant:<id>`.
    pub fn agent_type(&self) -> String {
        assistant_agent_type(&self.id)
    }

    pub fn grant(&self, public_key: &str) -> Option<&ShareGrant> {
        self.grants
            .iter()
            .find(|grant| grant.peer.public_key == public_key)
    }

    pub fn is_blocked(&self, public_key: &str) -> bool {
        self.blocked
            .iter()
            .any(|blocked| blocked.peer.public_key == public_key)
    }

    /// Whether a device with `device_id` holds a grant, under any key.
    pub fn is_granted_to_device_id(&self, device_id: &str) -> bool {
        self.grants
            .iter()
            .any(|grant| grant.peer.device_id == device_id)
    }

    fn validate(&self) -> Result<(), String> {
        if assistant_directory(&self.agent_type()) != Some(self.id.as_str()) {
            return Err(format!("'{}' does not name an assistant", self.id));
        }
        self.settings.validate()?;
        for (index, grant) in self.grants.iter().enumerate() {
            grant.peer.validate()?;
            if self.grants[..index]
                .iter()
                .any(|earlier| earlier.peer.public_key == grant.peer.public_key)
            {
                return Err("a device is granted more than once".to_string());
            }
        }
        for blocked in &self.blocked {
            blocked.peer.validate()?;
        }
        Ok(())
    }
}

/// Why a delegated task from a device is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ShareRefusal {
    #[error("assistant '{0}' is not shared on this device")]
    NotShared(String),
    #[error("this device has no access to assistant '{0}'; request it again")]
    NotGranted(String),
    #[error("the owner declined further requests from this device for assistant '{0}'")]
    Blocked(String),
}

#[derive(Debug, Error)]
pub enum ShareStoreError {
    #[error(transparent)]
    Store(#[from] JsonFileStoreError),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedAssistantsDocument {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    shares: Vec<SharedAssistant>,
}

/// Device-local store of the assistants this device shares.
#[derive(Debug, Clone)]
pub struct SharedAssistantStore {
    path: PathBuf,
}

impl SharedAssistantStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_product_home(path_manager: &PathManager) -> Self {
        Self::new(
            path_manager
                .product_home_dir()
                .join(DELEGATION_DIR)
                .join(SHARED_ASSISTANTS_FILE),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every share, read without the store lock for callers that cannot
    /// wait, such as the dispatch retention cleanup. Writes replace the file
    /// atomically, so the read sees one whole version.
    pub fn shares_now(&self) -> Result<Vec<SharedAssistant>, ShareStoreError> {
        let document: SharedAssistantsDocument = match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                ShareStoreError::Invalid(format!("shared assistants unreadable: {error}"))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                SharedAssistantsDocument::default()
            }
            Err(error) => {
                return Err(ShareStoreError::Invalid(format!(
                    "read shared assistants: {error}"
                )))
            }
        };
        Self::checked(document.shares)
    }

    /// Every share.
    pub async fn shares(&self) -> Result<Vec<SharedAssistant>, ShareStoreError> {
        let document: SharedAssistantsDocument = JsonFileStore
            .read_locked_optional(&self.path)
            .await?
            .unwrap_or_default();
        Self::checked(document.shares)
    }

    fn checked(shares: Vec<SharedAssistant>) -> Result<Vec<SharedAssistant>, ShareStoreError> {
        for share in &shares {
            share.validate().map_err(|problem| {
                ShareStoreError::Invalid(format!("shared assistant '{}': {problem}", share.id))
            })?;
        }
        Ok(shares)
    }

    pub async fn share_of(&self, id: &str) -> Result<Option<SharedAssistant>, ShareStoreError> {
        Ok(self
            .shares()
            .await?
            .into_iter()
            .find(|share| share.id == id))
    }

    /// The share `id` and the grant of `public_key` a task may run under,
    /// or why not.
    pub async fn authorize(
        &self,
        id: &str,
        public_key: &str,
    ) -> Result<Result<(SharedAssistant, ShareGrant), ShareRefusal>, ShareStoreError> {
        let Some(share) = self.share_of(id).await? else {
            return Ok(Err(ShareRefusal::NotShared(id.to_string())));
        };
        let public_key = public_key.trim().to_ascii_lowercase();
        if share.is_blocked(&public_key) {
            return Ok(Err(ShareRefusal::Blocked(share.id)));
        }
        match share.grant(&public_key).cloned() {
            Some(grant) => Ok(Ok((share, grant))),
            None => Ok(Err(ShareRefusal::NotGranted(share.id))),
        }
    }

    /// Shares the assistant in `assistants_base/<id>`. Full trust needs the
    /// owner to confirm its risk.
    pub async fn share(
        &self,
        assistants_base: &Path,
        id: &str,
        settings: ShareSettings,
        confirmations: ShareConfirmations,
    ) -> Result<SharedAssistant, ShareStoreError> {
        if !assistants_base.join(id).is_dir() {
            return Err(ShareStoreError::Invalid(format!(
                "assistant '{id}' does not exist on this device"
            )));
        }
        require_full_trust_confirmation(DelegationTrust::Normal, settings, confirmations)?;
        let share = SharedAssistant {
            id: id.to_string(),
            settings,
            external_agent_usage_confirmed: confirmations.external_agent_usage,
            display_name: None,
            description: None,
            shared_at: now(),
            grants: Vec::new(),
            blocked: Vec::new(),
        };
        share.validate().map_err(ShareStoreError::Invalid)?;
        self.update(|shares| {
            if shares.iter().any(|existing| existing.id == share.id) {
                return Err(format!("assistant '{id}' is already shared"));
            }
            shares.push(share.clone());
            Ok(share.clone())
        })
        .await
    }

    /// Changes the settings of the share `id`.
    pub async fn change_settings(
        &self,
        id: &str,
        settings: ShareSettings,
        confirmations: ShareConfirmations,
    ) -> Result<SharedAssistant, ShareStoreError> {
        settings.validate().map_err(ShareStoreError::Invalid)?;
        self.update_share(id, |share| {
            require_full_trust_confirmation(share.settings.trust, settings, confirmations)
                .map_err(|error| error.to_string())?;
            share.settings = settings;
            share.external_agent_usage_confirmed |= confirmations.external_agent_usage;
            Ok(())
        })
        .await
    }

    /// Sets the name other devices see for the share `id`. An empty name
    /// clears it.
    pub async fn set_display_name(
        &self,
        id: &str,
        display_name: &str,
    ) -> Result<SharedAssistant, ShareStoreError> {
        let name = bounded_name(display_name);
        self.update_share(id, |share| {
            share.display_name = (!name.is_empty()).then_some(name);
            Ok(())
        })
        .await
    }

    /// Sets the description other devices see for the share `id`; an empty
    /// one clears it.
    pub async fn set_description(
        &self,
        id: &str,
        description: &str,
    ) -> Result<SharedAssistant, ShareStoreError> {
        let description = bounded_text(description, MAX_DESCRIPTION_CHARS);
        self.update_share(id, |share| {
            share.description = (!description.is_empty()).then_some(description);
            Ok(())
        })
        .await
    }

    /// Stops sharing `id`, withdrawing every grant. Returns the share it
    /// removed.
    pub async fn stop(&self, id: &str) -> Result<Option<SharedAssistant>, ShareStoreError> {
        self.update(|shares| {
            let index = shares.iter().position(|share| share.id == id);
            Ok(index.map(|index| shares.remove(index)))
        })
        .await
    }

    /// Grants the share `id` to `peer`. A grant to a blocked device lifts
    /// the block: the owner allowed it in person.
    pub async fn grant(&self, id: &str, peer: SharePeer) -> Result<ShareGrant, ShareStoreError> {
        peer.validate().map_err(ShareStoreError::Invalid)?;
        let mut granted = None;
        self.update_share(id, |share| {
            share
                .blocked
                .retain(|blocked| blocked.peer.public_key != peer.public_key);
            let grant = match share
                .grants
                .iter_mut()
                .find(|grant| grant.peer.public_key == peer.public_key)
            {
                Some(existing) => {
                    existing.peer = peer.clone();
                    existing.clone()
                }
                None => {
                    let grant = ShareGrant {
                        peer: peer.clone(),
                        granted_at: now(),
                        last_used_at: None,
                    };
                    share.grants.push(grant.clone());
                    grant
                }
            };
            granted = Some(grant);
            Ok(())
        })
        .await?;
        Ok(granted.expect("grant set"))
    }

    /// Withdraws the grant of `public_key`. Returns the grant it removed.
    pub async fn revoke(
        &self,
        id: &str,
        public_key: &str,
    ) -> Result<Option<ShareGrant>, ShareStoreError> {
        let mut removed = None;
        self.update_share(id, |share| {
            if let Some(index) = share
                .grants
                .iter()
                .position(|grant| grant.peer.public_key == public_key)
            {
                removed = Some(share.grants.remove(index));
            }
            Ok(())
        })
        .await?;
        Ok(removed)
    }

    /// Lets `public_key` ask for `id` again. Returns whether it was blocked.
    pub async fn unblock(&self, id: &str, public_key: &str) -> Result<bool, ShareStoreError> {
        let mut unblocked = false;
        self.update_share(id, |share| {
            let before = share.blocked.len();
            share
                .blocked
                .retain(|blocked| blocked.peer.public_key != public_key);
            unblocked = share.blocked.len() != before;
            Ok(())
        })
        .await?;
        Ok(unblocked)
    }

    /// Blocks `peer` from requesting `id` again, withdrawing its grant.
    pub async fn block(&self, id: &str, peer: SharePeer) -> Result<(), ShareStoreError> {
        peer.validate().map_err(ShareStoreError::Invalid)?;
        self.update_share(id, |share| {
            share
                .grants
                .retain(|grant| grant.peer.public_key != peer.public_key);
            if !share.is_blocked(&peer.public_key) {
                share.blocked.push(BlockedDevice {
                    peer: peer.clone(),
                    blocked_at: now(),
                });
            }
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// Notes that the grant of `public_key` was used now. A missing share or
    /// grant changes nothing.
    pub async fn record_use(&self, id: &str, public_key: &str) -> Result<(), ShareStoreError> {
        self.update(|shares| {
            if let Some(grant) = shares
                .iter_mut()
                .find(|share| share.id == id)
                .and_then(|share| {
                    share
                        .grants
                        .iter_mut()
                        .find(|grant| grant.peer.public_key == public_key)
                })
            {
                grant.last_used_at = Some(now());
            }
            Ok(())
        })
        .await
    }

    async fn update_share(
        &self,
        id: &str,
        change: impl FnOnce(&mut SharedAssistant) -> Result<(), String>,
    ) -> Result<SharedAssistant, ShareStoreError> {
        self.update(|shares| {
            let share = shares
                .iter_mut()
                .find(|share| share.id == id)
                .ok_or_else(|| format!("assistant '{id}' is not shared"))?;
            change(share)?;
            share.validate()?;
            Ok(share.clone())
        })
        .await
    }

    /// Applies `change` under the store lock. A file that cannot be read is
    /// left as it is.
    async fn update<R>(
        &self,
        change: impl FnOnce(&mut Vec<SharedAssistant>) -> Result<R, String>,
    ) -> Result<R, ShareStoreError> {
        // Refuse before writing when the stored file is unreadable.
        self.shares().await?;
        let (outcome, _) = JsonFileStore
            .update_locked(
                &self.path,
                SharedAssistantsDocument::default(),
                |document: &mut SharedAssistantsDocument| {
                    let mut shares = document.shares.clone();
                    let outcome = change(&mut shares);
                    if outcome.is_ok() {
                        document.schema_version = SCHEMA_VERSION;
                        document.shares = shares;
                    }
                    outcome
                },
            )
            .await?;
        outcome.map_err(ShareStoreError::Invalid)
    }
}

fn require_full_trust_confirmation(
    current: DelegationTrust,
    settings: ShareSettings,
    confirmations: ShareConfirmations,
) -> Result<(), ShareStoreError> {
    if settings.trust == DelegationTrust::Full
        && current != DelegationTrust::Full
        && !confirmations.full_trust_risk
    {
        return Err(ShareStoreError::Invalid(
            "full trust lets the users of this assistant act as you; confirm the risk to turn it on"
                .to_string(),
        ));
    }
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
