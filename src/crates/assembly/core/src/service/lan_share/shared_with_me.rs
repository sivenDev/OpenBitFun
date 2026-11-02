//! Assistants other devices shared with this one.
//!
//! Kept device-local at `<product home>/lan-share/shared-with-me.json`. Each
//! entry pins the owner's sharing key: a device that later shows the same id
//! with another key is not that owner. A removal the owner has not heard of
//! yet is kept until it is delivered. A file this build cannot read is
//! reported and never overwritten.

use std::path::{Path, PathBuf};

use openbitfun_services_core::json_store::{JsonFileStore, JsonFileStoreError};
use serde::{Deserialize, Serialize};

use super::protocol::{Access, CatalogAssistant};
use super::shared_assistants::SharePeer;
use crate::infrastructure::PathManager;

const LAN_SHARE_DIR: &str = "lan-share";
const SHARED_WITH_ME_FILE: &str = "shared-with-me.json";
const SCHEMA_VERSION: u32 = 1;

/// Whether this device may still use an assistant shared with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SharedWithMeState {
    Available,
    /// The owner withdrew access or stopped sharing.
    Revoked,
}

/// One assistant shared with this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedWithMe {
    /// The owner's device, with its sharing key pinned.
    pub owner: SharePeer,
    pub assistant_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub granted_at: String,
    pub state: SharedWithMeState,
    /// Where the owner was last reached (`ip:port`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_address: Option<String>,
}

/// A removal the owner has not received yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRemoval {
    pub owner: SharePeer,
    pub assistant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_address: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedWithMeDocument {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    assistants: Vec<SharedWithMe>,
    #[serde(default)]
    pending_removals: Vec<PendingRemoval>,
}

#[derive(Debug, thiserror::Error)]
pub enum SharedWithMeError {
    #[error(transparent)]
    Store(#[from] JsonFileStoreError),
}

/// Device-local store of the assistants shared with this device.
#[derive(Debug, Clone)]
pub struct SharedWithMeStore {
    path: PathBuf,
}

impl SharedWithMeStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_product_home(path_manager: &PathManager) -> Self {
        Self::new(
            path_manager
                .product_home_dir()
                .join(LAN_SHARE_DIR)
                .join(SHARED_WITH_ME_FILE),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn assistants(&self) -> Result<Vec<SharedWithMe>, SharedWithMeError> {
        Ok(self.read().await?.assistants)
    }

    pub async fn pending_removals(&self) -> Result<Vec<PendingRemoval>, SharedWithMeError> {
        Ok(self.read().await?.pending_removals)
    }

    /// Records that `owner` granted `assistant`, replacing an earlier entry
    /// for the same owner key and assistant.
    pub async fn add_grant(
        &self,
        owner: SharePeer,
        assistant: &CatalogAssistant,
        address: Option<String>,
    ) -> Result<SharedWithMe, SharedWithMeError> {
        let entry = SharedWithMe {
            owner,
            assistant_id: assistant.id.clone(),
            name: assistant.name.clone(),
            description: assistant.description.clone(),
            granted_at: chrono::Utc::now().to_rfc3339(),
            state: SharedWithMeState::Available,
            last_address: address,
        };
        let added = entry.clone();
        self.update(move |document| {
            document.pending_removals.retain(|removal| {
                !(removal.owner.public_key == entry.owner.public_key
                    && removal.assistant_id == entry.assistant_id)
            });
            document.assistants.retain(|existing| {
                !(existing.owner.public_key == entry.owner.public_key
                    && existing.assistant_id == entry.assistant_id)
            });
            document.assistants.push(entry);
        })
        .await?;
        Ok(added)
    }

    /// Marks `assistant_id` of `owner_key` revoked. Returns whether an
    /// available entry changed.
    pub async fn mark_revoked(
        &self,
        owner_key: &str,
        assistant_id: &str,
    ) -> Result<bool, SharedWithMeError> {
        let owner_key = owner_key.to_ascii_lowercase();
        let assistant_id = assistant_id.to_string();
        self.update(move |document| {
            let mut changed = false;
            for entry in &mut document.assistants {
                if entry.owner.public_key == owner_key
                    && entry.assistant_id == assistant_id
                    && entry.state == SharedWithMeState::Available
                {
                    entry.state = SharedWithMeState::Revoked;
                    changed = true;
                }
            }
            changed
        })
        .await
    }

    /// Brings the entries of `owner_key` in line with the owner's catalog:
    /// an assistant no longer granted is revoked, names follow the owner,
    /// and the owner's address is remembered. Returns whether anything
    /// changed.
    pub async fn sync_with_catalog(
        &self,
        owner_key: &str,
        catalog: &[CatalogAssistant],
        address: Option<String>,
    ) -> Result<bool, SharedWithMeError> {
        let owner_key = owner_key.to_ascii_lowercase();
        let catalog = catalog.to_vec();
        self.update(move |document| {
            let mut changed = false;
            for entry in &mut document.assistants {
                if entry.owner.public_key != owner_key {
                    continue;
                }
                let listed = catalog
                    .iter()
                    .find(|assistant| assistant.id == entry.assistant_id);
                match listed {
                    Some(assistant) if assistant.access == Access::Granted => {
                        if entry.name != assistant.name
                            || entry.description != assistant.description
                        {
                            entry.name = assistant.name.clone();
                            entry.description = assistant.description.clone();
                            changed = true;
                        }
                    }
                    _ if entry.state == SharedWithMeState::Available => {
                        entry.state = SharedWithMeState::Revoked;
                        changed = true;
                    }
                    _ => {}
                }
                if address.is_some() && entry.last_address != address {
                    entry.last_address = address.clone();
                    changed = true;
                }
            }
            changed
        })
        .await
    }

    /// Removes `assistant_id` of `owner_key` from this device. When
    /// `notify_owner`, the removal is kept until the owner hears of it.
    /// Returns the removed entry.
    pub async fn remove(
        &self,
        owner_key: &str,
        assistant_id: &str,
        notify_owner: bool,
    ) -> Result<Option<SharedWithMe>, SharedWithMeError> {
        let owner_key = owner_key.to_ascii_lowercase();
        let assistant_id = assistant_id.to_string();
        self.update(move |document| {
            let index = document.assistants.iter().position(|entry| {
                entry.owner.public_key == owner_key && entry.assistant_id == assistant_id
            })?;
            let removed = document.assistants.remove(index);
            if notify_owner && removed.state == SharedWithMeState::Available {
                document.pending_removals.push(PendingRemoval {
                    owner: removed.owner.clone(),
                    assistant_id: removed.assistant_id.clone(),
                    last_address: removed.last_address.clone(),
                });
            }
            Some(removed)
        })
        .await
    }

    /// Forgets a removal the owner received.
    pub async fn removal_delivered(
        &self,
        owner_key: &str,
        assistant_id: &str,
    ) -> Result<(), SharedWithMeError> {
        let owner_key = owner_key.to_ascii_lowercase();
        let assistant_id = assistant_id.to_string();
        self.update(move |document| {
            document.pending_removals.retain(|removal| {
                !(removal.owner.public_key == owner_key && removal.assistant_id == assistant_id)
            });
        })
        .await
    }

    async fn read(&self) -> Result<SharedWithMeDocument, SharedWithMeError> {
        Ok(JsonFileStore
            .read_locked_optional(&self.path)
            .await?
            .unwrap_or_default())
    }

    /// Applies `change` under the store lock. A file that cannot be read is
    /// left as it is.
    async fn update<R>(
        &self,
        change: impl FnOnce(&mut SharedWithMeDocument) -> R,
    ) -> Result<R, SharedWithMeError> {
        self.read().await?;
        let (outcome, _) = JsonFileStore
            .update_locked(
                &self.path,
                SharedWithMeDocument::default(),
                |document: &mut SharedWithMeDocument| {
                    document.schema_version = SCHEMA_VERSION;
                    change(document)
                },
            )
            .await?;
        Ok(outcome)
    }
}
