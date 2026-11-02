//! Bonjour advertising and browsing of sharing devices.
//!
//! The advertisement carries only what a peer needs before connecting: the
//! protocol version, the device id, its names, and the full public sharing
//! key, which the connection pins. The device id must belong to that key. What a device shares is read over the channel instead.
//! Advertising and browsing run only while a caller asks for them.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use super::identity::{clean_name, device_id_of, fingerprint_of};
use super::{is_sharing_address, PROTOCOL_VERSION, SERVICE_TYPE};

const KEY_VERSION: &str = "v";
const KEY_DEVICE_ID: &str = "id";
const KEY_DEVICE_NAME: &str = "name";
const KEY_DISPLAY_NAME: &str = "owner";
const KEY_PUBLIC_KEY: &str = "pk";

/// What this device advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertisement {
    pub device_id: String,
    pub device_name: String,
    pub display_name: String,
    pub public_key: [u8; 32],
    pub port: u16,
}

/// A sharing device seen on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub device_name: String,
    pub display_name: String,
    /// The advertised sharing key; connect with it pinned.
    pub public_key: [u8; 32],
    /// The short code shown to people, derived from `public_key`.
    pub fingerprint: String,
    /// The protocol version the device advertises. A version this build does
    /// not speak is still listed, so the product can say it needs an update.
    pub protocol_version: u32,
    pub addresses: Vec<Ipv4Addr>,
    pub port: u16,
}

impl DiscoveredPeer {
    pub fn is_compatible(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
    }
}

/// A change in the set of sharing devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Found(DiscoveredPeer),
    Lost { device_id: String },
}

/// The Bonjour responder and browser of this device.
pub struct Discovery {
    daemon: ServiceDaemon,
    own_device_id: String,
    allow_loopback: bool,
    advertised: Mutex<Option<String>>,
}

impl Discovery {
    /// Starts the Bonjour daemon. `allow_loopback` lets single-machine tools
    /// and tests see devices on 127.0.0.1.
    pub fn new(own_device_id: &str, allow_loopback: bool) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("start the Bonjour daemon")?;
        Ok(Self {
            daemon,
            own_device_id: own_device_id.to_string(),
            allow_loopback,
            advertised: Mutex::new(None),
        })
    }

    /// Advertises this device, replacing an earlier advertisement.
    pub fn advertise(&self, advertisement: &Advertisement) -> Result<()> {
        self.stop_advertising()?;
        let addresses = sharing_ipv4_addresses(self.allow_loopback);
        if addresses.is_empty() {
            anyhow::bail!("this device has no private network address to advertise on");
        }
        let properties = [
            (KEY_VERSION, PROTOCOL_VERSION.to_string()),
            (KEY_DEVICE_ID, advertisement.device_id.clone()),
            (KEY_DEVICE_NAME, advertisement.device_name.clone()),
            (KEY_DISPLAY_NAME, advertisement.display_name.clone()),
            (KEY_PUBLIC_KEY, BASE64.encode(advertisement.public_key)),
        ];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &advertisement.device_id,
            &format!("{}.local.", advertisement.device_id),
            addresses
                .iter()
                .map(|ip| IpAddr::V4(*ip))
                .collect::<Vec<_>>()
                .as_slice(),
            advertisement.port,
            &properties[..],
        )
        .context("build the Bonjour advertisement")?;
        let fullname = info.get_fullname().to_string();
        self.daemon
            .register(info)
            .context("register the Bonjour advertisement")?;
        *self.advertised.lock().expect("advertisement lock") = Some(fullname);
        Ok(())
    }

    /// Withdraws this device's advertisement, if any.
    pub fn stop_advertising(&self) -> Result<()> {
        let fullname = self.advertised.lock().expect("advertisement lock").take();
        if let Some(fullname) = fullname {
            self.daemon
                .unregister(&fullname)
                .context("withdraw the Bonjour advertisement")?;
        }
        Ok(())
    }

    /// Starts browsing. Browsing stops when the returned browser is dropped.
    pub fn browse(&self) -> Result<DiscoveryBrowser> {
        let receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .context("browse for sharing devices")?;
        let (sender, events) = mpsc::channel(64);
        let own = self.own_device_id.clone();
        let allow_loopback = self.allow_loopback;
        let task = tokio::spawn(async move {
            let mut peers = PeerSet::default();
            while let Ok(event) = receiver.recv_async().await {
                let event = match event {
                    ServiceEvent::ServiceResolved(service) => {
                        let properties: HashMap<String, String> = service
                            .get_properties()
                            .iter()
                            .map(|property| {
                                (property.key().to_string(), property.val_str().to_string())
                            })
                            .collect();
                        let addresses: Vec<Ipv4Addr> =
                            service.get_addresses_v4().into_iter().collect();
                        match parse_advertisement(
                            service.get_fullname(),
                            &properties,
                            &addresses,
                            service.get_port(),
                            allow_loopback,
                        ) {
                            Some(peer) if peer.device_id != own => DiscoveryEvent::Found(peer),
                            Some(_) => continue,
                            None => {
                                log::debug!(
                                    "Ignored a malformed LAN sharing advertisement: {}",
                                    service.get_fullname()
                                );
                                continue;
                            }
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => match instance_of(&fullname) {
                        Some(device_id) if device_id != own => DiscoveryEvent::Lost { device_id },
                        _ => continue,
                    },
                    _ => continue,
                };
                let Some(event) = peers.apply(event) else {
                    continue;
                };
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(DiscoveryBrowser {
            daemon: self.daemon.clone(),
            events,
            task,
        })
    }

    /// Stops advertising and browsing and shuts the daemon down.
    pub fn shutdown(&self) {
        let _ = self.stop_advertising();
        let _ = self.daemon.shutdown();
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A running browse.
pub struct DiscoveryBrowser {
    daemon: ServiceDaemon,
    events: mpsc::Receiver<DiscoveryEvent>,
    task: tokio::task::JoinHandle<()>,
}

impl DiscoveryBrowser {
    /// The next change; `None` once browsing has ended.
    pub async fn next(&mut self) -> Option<DiscoveryEvent> {
        self.events.recv().await
    }
}

impl Drop for DiscoveryBrowser {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        self.task.abort();
    }
}

/// The devices currently seen. Bonjour resolves a device again whenever its
/// records refresh; only a new or changed device is reported.
#[derive(Debug, Default)]
pub struct PeerSet {
    peers: HashMap<String, DiscoveredPeer>,
}

impl PeerSet {
    /// Records `event` and returns it when it changes the set.
    pub fn apply(&mut self, event: DiscoveryEvent) -> Option<DiscoveryEvent> {
        match &event {
            DiscoveryEvent::Found(peer) => {
                if self.peers.get(&peer.device_id) == Some(peer) {
                    return None;
                }
                self.peers.insert(peer.device_id.clone(), peer.clone());
            }
            DiscoveryEvent::Lost { device_id } => {
                self.peers.remove(device_id)?;
            }
        }
        Some(event)
    }
}

/// Parses one resolved advertisement; `None` when it is malformed.
pub fn parse_advertisement(
    fullname: &str,
    properties: &HashMap<String, String>,
    addresses: &[Ipv4Addr],
    port: u16,
    allow_loopback: bool,
) -> Option<DiscoveredPeer> {
    let instance = instance_of(fullname)?;
    let device_id = properties.get(KEY_DEVICE_ID)?;
    let public_key: [u8; 32] = BASE64
        .decode(properties.get(KEY_PUBLIC_KEY)?)
        .ok()?
        .try_into()
        .ok()?;
    if device_id != &instance || device_id != &device_id_of(&public_key) {
        return None;
    }
    let protocol_version = properties.get(KEY_VERSION)?.parse::<u32>().ok()?;
    let name = |key: &str| {
        let value = properties.get(key)?;
        (value.chars().count() <= super::MAX_NAME_CHARS)
            .then(|| clean_name(value))
            .flatten()
    };
    let mut addresses: Vec<Ipv4Addr> = addresses
        .iter()
        .copied()
        .filter(|ip| is_sharing_address(IpAddr::V4(*ip), allow_loopback))
        .collect();
    addresses.sort();
    if addresses.is_empty() || port == 0 {
        return None;
    }
    Some(DiscoveredPeer {
        device_id: device_id.clone(),
        device_name: name(KEY_DEVICE_NAME)?,
        display_name: name(KEY_DISPLAY_NAME)?,
        public_key,
        fingerprint: fingerprint_of(&public_key),
        protocol_version,
        addresses,
        port,
    })
}

/// The instance label of a full service name, which is the device id.
fn instance_of(fullname: &str) -> Option<String> {
    let instance = fullname.strip_suffix(&format!(".{SERVICE_TYPE}"))?;
    (!instance.is_empty() && !instance.contains('.')).then(|| instance.to_string())
}

/// The private IPv4 addresses of this device's network interfaces.
pub fn sharing_ipv4_addresses(allow_loopback: bool) -> Vec<Ipv4Addr> {
    let mut addresses: Vec<Ipv4Addr> = local_ip_address::list_afinet_netifas()
        .map(|interfaces| {
            interfaces
                .into_iter()
                .filter_map(|(_, ip)| match ip {
                    IpAddr::V4(v4) if is_sharing_address(ip, false) => Some(v4),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if allow_loopback {
        addresses.push(Ipv4Addr::LOCALHOST);
    }
    addresses.sort();
    addresses.dedup();
    addresses
}
