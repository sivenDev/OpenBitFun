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

/// The port a sharing device listens on unless another program holds it.
/// People on another subnet connect with the address alone.
pub const DEFAULT_PORT: u16 = 47652;

/// Longest device or display name accepted from the network or stored.
pub const MAX_NAME_CHARS: usize = 64;

/// Why text someone typed is not an address of a sharing device.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("enter an address such as 192.168.1.20, optionally with a port")]
    Format,
    /// A name rather than an address.
    #[error("enter the device's address, not its name")]
    HostName,
    #[error("only IPv4 addresses are supported for now")]
    Ipv6,
    /// A public, loopback, or other address outside the local network.
    #[error("{0} is not an address on the local network")]
    NotLocal(String),
}

/// Reads an address someone typed: `ip` or `ip:port`, the port defaulting to
/// [`DEFAULT_PORT`]. Only addresses a sharing peer may use are accepted, as
/// [`is_sharing_address`] decides.
pub fn parse_peer_address(
    input: &str,
    allow_loopback: bool,
) -> Result<std::net::SocketAddr, AddressError> {
    use std::net::{IpAddr, SocketAddr};
    let input = input.trim();
    let address = if let Ok(address) = input.parse::<SocketAddr>() {
        address
    } else if let Ok(ip) = input.parse::<IpAddr>() {
        SocketAddr::new(ip, DEFAULT_PORT)
    } else if input.chars().any(|c| c.is_ascii_alphabetic()) && !input.contains(char::is_whitespace)
    {
        return Err(if input.contains("::") || input.starts_with('[') {
            AddressError::Ipv6
        } else {
            AddressError::HostName
        });
    } else {
        return Err(AddressError::Format);
    };
    if address.is_ipv6() {
        return Err(AddressError::Ipv6);
    }
    if address.port() == 0 {
        return Err(AddressError::Format);
    }
    if !is_sharing_address(address.ip(), allow_loopback) {
        return Err(AddressError::NotLocal(address.ip().to_string()));
    }
    Ok(address)
}

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
