//! Sharing devices this device added by address.
//!
//! Bonjour does not cross subnets, so a device on another subnet is added by
//! the address its owner reads out, after the person compared the device
//! codes. Kept device-local at `<product home>/lan-share/added-devices.json`;
//! each entry pins the device's sharing key, so whoever answers at the
//! address later must hold that key. A file this build cannot read is
//! reported and never overwritten.

use std::path::{Path, PathBuf};

use openbitfun_services_core::json_store::{JsonFileStore, JsonFileStoreError};
use serde::{Deserialize, Serialize};

use super::shared_assistants::SharePeer;
use crate::infrastructure::PathManager;

const LAN_SHARE_DIR: &str = "lan-share";
const ADDED_DEVICES_FILE: &str = "added-devices.json";
const SCHEMA_VERSION: u32 = 1;

/// A device added by address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedDevice {
    /// The device, with its sharing key pinned.
    pub peer: SharePeer,
    /// Where it answers (`ip:port`).
    pub address: String,
    pub added_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddedDevicesDocument {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    devices: Vec<AddedDevice>,
}

#[derive(Debug, thiserror::Error)]
pub enum AddedDevicesError {
    #[error(transparent)]
    Store(#[from] JsonFileStoreError),
}

/// Device-local store of the devices added by address.
#[derive(Debug, Clone)]
pub struct AddedDeviceStore {
    path: PathBuf,
}

impl AddedDeviceStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_product_home(path_manager: &PathManager) -> Self {
        Self::new(
            path_manager
                .product_home_dir()
                .join(LAN_SHARE_DIR)
                .join(ADDED_DEVICES_FILE),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn devices(&self) -> Result<Vec<AddedDevice>, AddedDevicesError> {
        Ok(self.read().await?.devices)
    }

    /// The device holding `public_key`, if added.
    pub async fn device_with_key(
        &self,
        public_key: &str,
    ) -> Result<Option<AddedDevice>, AddedDevicesError> {
        let public_key = public_key.to_ascii_lowercase();
        Ok(self
            .devices()
            .await?
            .into_iter()
            .find(|device| device.peer.public_key == public_key))
    }

    /// Adds `peer` at `address`, or moves it there when already added.
    pub async fn add(
        &self,
        peer: SharePeer,
        address: String,
    ) -> Result<AddedDevice, AddedDevicesError> {
        self.update(move |document| {
            match document
                .devices
                .iter_mut()
                .find(|device| device.peer.public_key == peer.public_key)
            {
                Some(device) => {
                    device.peer = peer;
                    device.address = address;
                    device.clone()
                }
                None => {
                    let device = AddedDevice {
                        peer,
                        address,
                        added_at: chrono::Utc::now().to_rfc3339(),
                    };
                    document.devices.push(device.clone());
                    device
                }
            }
        })
        .await
    }

    /// Moves the device holding `public_key` to `address`. Returns whether
    /// it changed.
    pub async fn set_address(
        &self,
        public_key: &str,
        address: String,
    ) -> Result<bool, AddedDevicesError> {
        let public_key = public_key.to_ascii_lowercase();
        self.update(move |document| {
            match document
                .devices
                .iter_mut()
                .find(|device| device.peer.public_key == public_key)
            {
                Some(device) if device.address != address => {
                    device.address = address;
                    true
                }
                _ => false,
            }
        })
        .await
    }

    /// Forgets the device `device_id`. Returns the removed entry.
    pub async fn remove(&self, device_id: &str) -> Result<Option<AddedDevice>, AddedDevicesError> {
        let device_id = device_id.to_string();
        self.update(move |document| {
            let index = document
                .devices
                .iter()
                .position(|device| device.peer.device_id == device_id)?;
            Some(document.devices.remove(index))
        })
        .await
    }

    async fn read(&self) -> Result<AddedDevicesDocument, AddedDevicesError> {
        Ok(JsonFileStore
            .read_locked_optional(&self.path)
            .await?
            .unwrap_or_default())
    }

    /// Applies `change` under the store lock. A file that cannot be read is
    /// left as it is.
    async fn update<R>(
        &self,
        change: impl FnOnce(&mut AddedDevicesDocument) -> R,
    ) -> Result<R, AddedDevicesError> {
        self.read().await?;
        let (outcome, _) = JsonFileStore
            .update_locked(
                &self.path,
                AddedDevicesDocument::default(),
                |document: &mut AddedDevicesDocument| {
                    document.schema_version = SCHEMA_VERSION;
                    change(document)
                },
            )
            .await?;
        Ok(outcome)
    }
}
