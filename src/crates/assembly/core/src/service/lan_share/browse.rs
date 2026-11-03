//! What the Discover panel shows: the sharing devices on the local network,
//! their shared assistants, and where this device stands with each.
//!
//! Pure state. The host feeds it what Bonjour finds and what each device's
//! catalog says; a device whose protocol this build does not speak is listed
//! as needing an update and never contacted.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};

use openbitfun_services_integrations::lan_share::DiscoveredPeer;
use serde::Serialize;

use super::protocol::{Access, CatalogAssistant};

/// One assistant as the Discover panel lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredAssistant {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub access: Access,
}

/// Whether this device can talk to a discovered device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum DeviceStatus {
    /// Its catalog is being read.
    Connecting,
    Ready,
    /// One side needs a newer version.
    NeedsUpdate {
        message: String,
    },
    Unreachable {
        message: String,
    },
}

/// One sharing device on the local network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    pub device_id: String,
    pub device_name: String,
    pub display_name: String,
    pub fingerprint: String,
    /// The advertised sharing key, 64 hex digits.
    pub public_key: String,
    pub status: DeviceStatus,
    pub assistants: Vec<DiscoveredAssistant>,
}

#[derive(Debug, Default)]
pub struct BrowseState {
    peers: BTreeMap<String, DiscoveredPeer>,
    devices: BTreeMap<String, DiscoveredDevice>,
}

impl BrowseState {
    /// Lists a device Bonjour found. Returns whether this device should read
    /// its catalog: a compatible device it did not know with this key.
    pub fn found(&mut self, peer: DiscoveredPeer) -> bool {
        let key = hex::encode(peer.public_key);
        let known = self
            .devices
            .get(&peer.device_id)
            .is_some_and(|device| device.public_key == key && device.status == DeviceStatus::Ready);
        let status = if peer.is_compatible() {
            DeviceStatus::Connecting
        } else {
            DeviceStatus::NeedsUpdate {
                message: format!(
                    "{} uses a different version; update both devices to the latest version",
                    peer.device_name
                ),
            }
        };
        let read_catalog = peer.is_compatible() && !known;
        if !known {
            self.devices.insert(
                peer.device_id.clone(),
                DiscoveredDevice {
                    device_id: peer.device_id.clone(),
                    device_name: peer.device_name.clone(),
                    display_name: peer.display_name.clone(),
                    fingerprint: peer.fingerprint.clone(),
                    public_key: key,
                    status,
                    assistants: Vec::new(),
                },
            );
        }
        self.peers.insert(peer.device_id.clone(), peer);
        read_catalog
    }

    pub fn lost(&mut self, device_id: &str) -> Option<DiscoveredPeer> {
        self.devices.remove(device_id);
        self.peers.remove(device_id)
    }

    /// The devices found whose catalog this device can read.
    pub fn readable_peers(&self) -> Vec<DiscoveredPeer> {
        self.peers
            .values()
            .filter(|peer| peer.is_compatible())
            .cloned()
            .collect()
    }

    pub fn peer(&self, device_id: &str) -> Option<&DiscoveredPeer> {
        self.peers.get(device_id)
    }

    /// The device that holds `public_key`, if listed.
    pub fn peer_with_key(&self, public_key: &str) -> Option<&DiscoveredPeer> {
        self.peers
            .values()
            .find(|peer| hex::encode(peer.public_key) == public_key)
    }

    /// Records the catalog of `device_id`.
    pub fn catalog(&mut self, device_id: &str, assistants: Vec<CatalogAssistant>) {
        if let Some(device) = self.devices.get_mut(device_id) {
            device.status = DeviceStatus::Ready;
            device.assistants = assistants
                .into_iter()
                .map(|assistant| DiscoveredAssistant {
                    id: assistant.id,
                    name: assistant.name,
                    description: assistant.description,
                    access: assistant.access,
                })
                .collect();
        }
    }

    pub fn failed(&mut self, device_id: &str, status: DeviceStatus) {
        if let Some(device) = self.devices.get_mut(device_id) {
            device.status = status;
            device.assistants.clear();
        }
    }

    /// Changes where this device stands with `assistant_id` of the device
    /// holding `public_key`.
    pub fn set_access(&mut self, public_key: &str, assistant_id: &str, access: Access) -> bool {
        let Some(device) = self
            .devices
            .values_mut()
            .find(|device| device.public_key == public_key)
        else {
            return false;
        };
        match device
            .assistants
            .iter_mut()
            .find(|assistant| assistant.id == assistant_id)
        {
            Some(assistant) if assistant.access != access => {
                assistant.access = access;
                true
            }
            _ => false,
        }
    }

    /// Every listed device, by device name.
    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        let mut devices: Vec<_> = self.devices.values().cloned().collect();
        devices.sort_by(|left, right| {
            left.device_name
                .to_lowercase()
                .cmp(&right.device_name.to_lowercase())
                .then_with(|| left.device_id.cmp(&right.device_id))
        });
        devices
    }

    pub fn clear(&mut self) {
        self.peers.clear();
        self.devices.clear();
    }
}

/// The addresses to try for `peer`, in its advertised order.
pub fn peer_addresses(peer: &DiscoveredPeer) -> Vec<SocketAddr> {
    peer.addresses
        .iter()
        .map(|ip: &Ipv4Addr| SocketAddr::from((*ip, peer.port)))
        .collect()
}
