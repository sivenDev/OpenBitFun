use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

use super::discovery::{parse_advertisement, PeerSet};
use super::handshake::{FrameOpener, FrameSealer, Hello, LocalHandshake};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use super::identity::{device_id_of, fingerprint_of, is_fingerprint};
use super::*;

fn identity(name: &str) -> ShareIdentity {
    ShareIdentity::generate(name, name).unwrap()
}

#[test]
fn identity_persists_with_owner_only_permissions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lan-share").join("identity.json");

    let created = ShareIdentity::load_or_create(&path).unwrap();
    let loaded = ShareIdentity::load_or_create(&path).unwrap();

    assert_eq!(created.device_id(), loaded.device_id());
    assert_eq!(created.public_key(), loaded.public_key());
    assert_eq!(created.display_name(), loaded.display_name());
    assert!(is_fingerprint(&loaded.fingerprint()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn display_name_change_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("identity.json");
    let mut identity = ShareIdentity::load_or_create(&path).unwrap();

    identity.set_display_name("  Ada  ").unwrap();
    identity.save(&path).unwrap();

    assert_eq!(
        ShareIdentity::load_or_create(&path).unwrap().display_name(),
        "Ada"
    );
    assert!(identity.set_display_name("   ").is_err());
}

#[test]
fn unreadable_identity_is_reported_and_left_in_place() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("identity.json");
    std::fs::write(&path, b"{not json").unwrap();

    let error = ShareIdentity::load_or_create(&path).unwrap_err();

    assert!(format!("{error:#}").contains("unreadable"));
    assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
}

#[test]
fn fingerprints_are_stable_short_codes() {
    let a = identity("a");
    let b = identity("b");
    assert_eq!(a.fingerprint(), fingerprint_of(&a.public_key()));
    assert_ne!(a.fingerprint(), b.fingerprint());
    assert!(is_fingerprint(&a.fingerprint()));
    assert!(!is_fingerprint("ABC1O0"));
    assert!(!is_fingerprint("ABCDE"));
}

#[test]
fn only_private_addresses_take_part() {
    assert!(is_sharing_address(
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)),
        false
    ));
    assert!(is_sharing_address(
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
        false
    ));
    assert!(is_sharing_address(
        IpAddr::V4(Ipv4Addr::new(172, 16, 21, 153)),
        false
    ));
    assert!(!is_sharing_address(
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
        false
    ));
    assert!(!is_sharing_address(IpAddr::V4(Ipv4Addr::LOCALHOST), false));
    assert!(is_sharing_address(IpAddr::V4(Ipv4Addr::LOCALHOST), true));
    assert!(!is_sharing_address("fe80::1".parse().unwrap(), true));
}

/// Keys of both sides of one handshake; `tamper` changes the hello text of A
/// on its way to B.
fn handshake_pair(
    tamper: impl FnOnce(String) -> String,
) -> (super::handshake::SessionKeys, super::handshake::SessionKeys) {
    let a = identity("a");
    let b = identity("b");
    let (ha, hb) = (LocalHandshake::new(), LocalHandshake::new());
    let text_a = serde_json::to_string(&ha.hello(&a, PROTOCOL_VERSION, &[])).unwrap();
    let text_b = serde_json::to_string(&hb.hello(&b, PROTOCOL_VERSION, &[])).unwrap();
    let seen_by_b = tamper(text_a.clone());
    let hello_a = serde_json::from_str::<Hello>(&seen_by_b)
        .unwrap()
        .check()
        .unwrap();
    let hello_b = serde_json::from_str::<Hello>(&text_b)
        .unwrap()
        .check()
        .unwrap();
    let keys_a = ha.finish(&a, &hello_b, true, &text_a, &text_b).unwrap();
    let keys_b = hb.finish(&b, &hello_a, false, &text_b, &seen_by_b).unwrap();
    (keys_a, keys_b)
}

fn session_pair() -> (FrameSealer, FrameOpener) {
    let (keys_a, keys_b) = handshake_pair(|text| text);
    (keys_a.sender, keys_b.receiver)
}

#[test]
fn a_hello_altered_in_transit_leaves_the_sides_without_a_common_key() {
    let (keys_a, keys_b) = handshake_pair(|text| {
        text.replace("\"capabilities\":[]", "\"capabilities\":[\"injected\"]")
    });
    let (mut sealer, mut opener) = (keys_a.sender, keys_b.receiver);
    let frame = sealer.seal(b"ready").unwrap();
    assert!(opener.open(&frame).is_err());
}

#[test]
fn frames_round_trip_between_the_two_sides() {
    let (mut sealer, mut opener) = session_pair();
    for message in [&b"one"[..], b"two", b""] {
        let frame = sealer.seal(message).unwrap();
        assert_eq!(opener.open(&frame).unwrap(), message);
    }
}

#[test]
fn tampered_frames_are_refused() {
    let (mut sealer, mut opener) = session_pair();
    let mut frame = sealer.seal(b"hello").unwrap();
    let last = frame.len() - 1;
    frame[last] ^= 1;
    assert!(opener.open(&frame).unwrap_err().contains("authentication"));
}

#[test]
fn replayed_and_reordered_frames_are_refused() {
    let (mut sealer, mut opener) = session_pair();
    let first = sealer.seal(b"first").unwrap();
    let second = sealer.seal(b"second").unwrap();

    assert!(opener.open(&second).unwrap_err().contains("out-of-order"));
    opener.open(&first).unwrap();
    assert!(opener.open(&first).unwrap_err().contains("out-of-order"));
    opener.open(&second).unwrap();
}

#[test]
fn a_frame_from_another_session_is_refused() {
    let (mut sealer, _) = session_pair();
    let (_, mut other_opener) = session_pair();
    let frame = sealer.seal(b"hello").unwrap();
    assert!(other_opener.open(&frame).is_err());
}

#[test]
fn a_device_cannot_handshake_with_its_own_identity() {
    let a = identity("a");
    let (one, two) = (LocalHandshake::new(), LocalHandshake::new());
    let hello = two.hello(&a, PROTOCOL_VERSION, &[]).check().unwrap();
    assert!(one.finish(&a, &hello, true, "", "").is_err());
}

struct Advertised {
    device_id: String,
    public_key: [u8; 32],
}

fn advertised() -> Advertised {
    let identity = identity("mac");
    Advertised {
        device_id: identity.device_id().to_string(),
        public_key: identity.public_key(),
    }
}

fn advertisement(device: &Advertised) -> HashMap<String, String> {
    HashMap::from([
        ("v".to_string(), "1".to_string()),
        ("id".to_string(), device.device_id.clone()),
        ("name".to_string(), "Mac mini".to_string()),
        ("owner".to_string(), "shimo".to_string()),
        ("pk".to_string(), BASE64.encode(device.public_key)),
    ])
}

fn fullname(id: &str) -> String {
    format!("{id}.{SERVICE_TYPE}")
}

#[test]
fn a_well_formed_advertisement_is_parsed() {
    let device = advertised();
    let peer = parse_advertisement(
        &fullname(&device.device_id),
        &advertisement(&device),
        &[Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(192, 168, 1, 9)],
        5000,
        false,
    )
    .unwrap();
    assert_eq!(peer.device_id, device.device_id);
    assert_eq!(peer.public_key, device.public_key);
    assert_eq!(peer.fingerprint, fingerprint_of(&device.public_key));
    assert_eq!(peer.device_name, "Mac mini");
    assert_eq!(peer.display_name, "shimo");
    assert_eq!(peer.addresses, vec![Ipv4Addr::new(192, 168, 1, 9)]);
    assert!(peer.is_compatible());
}

#[test]
fn a_newer_protocol_is_listed_as_incompatible() {
    let device = advertised();
    let mut properties = advertisement(&device);
    properties.insert("v".to_string(), "2".to_string());
    let peer = parse_advertisement(
        &fullname(&device.device_id),
        &properties,
        &[Ipv4Addr::new(10, 0, 0, 3)],
        5000,
        false,
    )
    .unwrap();
    assert!(!peer.is_compatible());
}

#[test]
fn malformed_advertisements_are_dropped() {
    let device = advertised();
    let name = fullname(&device.device_id);
    let address = [Ipv4Addr::new(10, 0, 0, 3)];
    let parse = |name: &str, properties: &HashMap<String, String>, addresses: &[Ipv4Addr], port| {
        parse_advertisement(name, properties, addresses, port, false)
    };
    let with = |key: &str, value: &str| {
        let mut properties = advertisement(&device);
        properties.insert(key.to_string(), value.to_string());
        properties
    };
    let without = |key: &str| {
        let mut properties = advertisement(&device);
        properties.remove(key);
        properties
    };
    let good = advertisement(&device);

    for key in ["v", "id", "name", "owner", "pk"] {
        assert!(
            parse(&name, &without(key), &address, 5000).is_none(),
            "{key}"
        );
    }
    assert!(parse(&fullname("other"), &good, &address, 5000).is_none());
    assert!(parse(&name, &with("pk", "abc"), &address, 5000).is_none());
    assert!(parse(&name, &with("v", "x"), &address, 5000).is_none());
    assert!(parse(&name, &with("name", &"n".repeat(65)), &address, 5000).is_none());
    assert!(parse(&name, &with("owner", " "), &address, 5000).is_none());
    assert!(parse(&name, &good, &[], 5000).is_none());
    assert!(parse(&name, &good, &[Ipv4Addr::new(8, 8, 8, 8)], 5000).is_none());
    assert!(parse(&name, &good, &address, 0).is_none());
}

#[test]
fn an_advertised_key_must_own_the_advertised_device_id() {
    let device = advertised();
    let other = advertised();
    // Another device's id with this device's key, and the reverse.
    let mut properties = advertisement(&device);
    properties.insert("id".to_string(), other.device_id.clone());
    assert!(parse_advertisement(
        &fullname(&other.device_id),
        &properties,
        &[Ipv4Addr::new(10, 0, 0, 3)],
        5000,
        false
    )
    .is_none());
    assert_eq!(device.device_id, device_id_of(&device.public_key));
}

#[test]
fn a_hello_whose_device_id_is_not_its_key_is_refused() {
    let a = identity("a");
    let b = identity("b");
    let mut hello = LocalHandshake::new().hello(&a, PROTOCOL_VERSION, &[]);
    hello.device_id = b.device_id().to_string();
    assert!(hello.check().unwrap_err().contains("does not belong"));
}

#[test]
fn repeated_resolutions_report_a_device_once() {
    let device = advertised();
    let peer = parse_advertisement(
        &fullname(&device.device_id),
        &advertisement(&device),
        &[Ipv4Addr::new(10, 0, 0, 3)],
        5000,
        false,
    )
    .unwrap();
    let mut moved = peer.clone();
    moved.addresses = vec![Ipv4Addr::new(10, 0, 0, 4)];
    let lost = DiscoveryEvent::Lost {
        device_id: device.device_id.clone(),
    };
    let mut peers = PeerSet::default();

    assert!(peers.apply(DiscoveryEvent::Found(peer.clone())).is_some());
    assert!(peers.apply(DiscoveryEvent::Found(peer.clone())).is_none());
    assert!(peers.apply(DiscoveryEvent::Found(moved)).is_some());
    assert!(peers.apply(lost.clone()).is_some());
    assert!(peers.apply(lost).is_none());
    assert!(peers.apply(DiscoveryEvent::Found(peer)).is_some());
}
