//! Local-network assistant sharing primitives.
//!
//! Devices on the same local network find each other over Bonjour and talk
//! over a direct, end-to-end encrypted channel. Nothing here depends on an
//! account or a Relay: each device carries its own sharing identity.
//!
//! This module owns only the transport facts: the identity key pair, the
//! advertisement and its parsing, and the channel handshake and framing.
//! Who may request an assistant, grants, and confirmation belong to the
//! product layers built on top.

pub mod channel;
pub mod discovery;
pub mod handshake;
pub mod identity;

pub use channel::{
    connect, ChannelConfig, LanConnection, LanError, LanListener, LanPush, PeerIdentity,
    RequestHandler,
};
pub use discovery::{
    Advertisement, DiscoveredPeer, Discovery, DiscoveryBrowser, DiscoveryEvent, PeerSet,
};
pub use identity::{device_id_of, fingerprint_of, ShareIdentity};

/// The protocol version carried in the advertisement and the handshake.
pub const PROTOCOL_VERSION: u32 = 1;

/// The Bonjour service type devices advertise and browse.
pub const SERVICE_TYPE: &str = "_openbitfun._tcp.local.";

/// Longest device or display name accepted from the network or stored.
pub const MAX_NAME_CHARS: usize = 64;

/// Whether `ip` is an address a sharing peer may use: a private IPv4 range
/// (RFC 1918). Loopback is accepted only when the caller allows it, which
/// tests and single-machine tools do.
pub fn is_sharing_address(ip: std::net::IpAddr, allow_loopback: bool) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private() || (allow_loopback && v4.is_loopback()),
        std::net::IpAddr::V6(_) => false,
    }
}

#[cfg(test)]
mod tests;
