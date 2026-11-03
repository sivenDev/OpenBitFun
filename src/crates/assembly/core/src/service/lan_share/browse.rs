//! What the Discover panel shows: the sharing devices on the local network,
//! their shared assistants, and where this device stands with each.
//!
//! Pure state. The host feeds it what Bonjour finds, the devices added by
//! address, and what each device's catalog says; a device whose protocol
//! this build does not speak is listed as needing an update and never
//! contacted. A device both found and added is listed once, by its id, which
//! its sharing key decides.

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
    /// Whoever answers at the device's address holds another key.
    IdentityChanged {
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
    /// Set when the device was added by address: where it answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_address: Option<String>,
}

#[derive(Debug, Default)]
pub struct BrowseState {
    /// What Bonjour lists now.
    found: BTreeMap<String, DiscoveredPeer>,
    /// The devices added by address.
    added: BTreeMap<String, DiscoveredPeer>,
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
            self.list(&peer, key, status);
        }
        self.found.insert(peer.device_id.clone(), peer);
        read_catalog
    }

    /// Lists a device added by address, reachable at `peer.addresses` and
    /// `peer.port`. Its catalog is to be read unless it is listed as ready.
    pub fn add(&mut self, peer: DiscoveredPeer, address: String) -> bool {
        let key = hex::encode(peer.public_key);
        let known = self
            .devices
            .get(&peer.device_id)
            .is_some_and(|device| device.public_key == key && device.status == DeviceStatus::Ready);
        if !known {
            self.list(&peer, key, DeviceStatus::Connecting);
        }
        if let Some(device) = self.devices.get_mut(&peer.device_id) {
            device.added_address = Some(address);
        }
        self.added.insert(peer.device_id.clone(), peer);
        !known
    }

    fn list(&mut self, peer: &DiscoveredPeer, key: String, status: DeviceStatus) {
        let added_address = self
            .devices
            .get(&peer.device_id)
            .and_then(|device| device.added_address.clone());
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
                added_address,
            },
        );
    }

    /// Bonjour no longer lists `device_id`. A device added by address stays.
    pub fn lost(&mut self, device_id: &str) -> Option<DiscoveredPeer> {
        if !self.added.contains_key(device_id) {
            self.devices.remove(device_id);
        }
        self.found.remove(device_id)
    }

    /// Forgets that `device_id` was added by address. A device Bonjour lists
    /// stays. Returns whether the list changed.
    pub fn forget(&mut self, device_id: &str) -> bool {
        if self.added.remove(device_id).is_none() {
            return false;
        }
        if self.found.contains_key(device_id) {
            if let Some(device) = self.devices.get_mut(device_id) {
                device.added_address = None;
            }
        } else {
            self.devices.remove(device_id);
        }
        true
    }

    /// The devices listed whose catalog this device can read.
    pub fn readable_peers(&self) -> Vec<DiscoveredPeer> {
        self.peers()
            .filter(|peer| peer.is_compatible())
            .cloned()
            .collect()
    }

    /// The listed devices whose catalog could not be read because they did
    /// not answer: tried again while Discover is open, since one added by
    /// address is never announced when it comes back.
    pub fn unreachable_peers(&self) -> Vec<DiscoveredPeer> {
        self.peers()
            .filter(|peer| {
                peer.is_compatible()
                    && self.devices.get(&peer.device_id).is_some_and(|device| {
                        matches!(device.status, DeviceStatus::Unreachable { .. })
                    })
            })
            .cloned()
            .collect()
    }

    /// Every listed device once: as Bonjour lists it, or as it was added.
    fn peers(&self) -> impl Iterator<Item = &DiscoveredPeer> {
        self.found.values().chain(
            self.added
                .values()
                .filter(|peer| !self.found.contains_key(&peer.device_id)),
        )
    }

    pub fn peer(&self, device_id: &str) -> Option<&DiscoveredPeer> {
        self.found
            .get(device_id)
            .or_else(|| self.added.get(device_id))
    }

    /// Where `device_id` may answer: what Bonjour lists, then the address it
    /// was added at.
    pub fn addresses(&self, device_id: &str) -> Vec<SocketAddr> {
        let mut addresses = Vec::new();
        for peer in [self.found.get(device_id), self.added.get(device_id)]
            .into_iter()
            .flatten()
        {
            for address in peer_addresses(peer) {
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
            }
        }
        addresses
    }

    /// The id of the listed device that holds `public_key`.
    pub fn device_with_key(&self, public_key: &str) -> Option<String> {
        self.peers()
            .find(|peer| hex::encode(peer.public_key) == public_key)
            .map(|peer| peer.device_id.clone())
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
        self.found.clear();
        self.added.clear();
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
