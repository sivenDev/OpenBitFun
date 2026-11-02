//! The channel handshake and frame encryption.
//!
//! Each side sends a plaintext hello with its static sharing key and a fresh
//! ephemeral key. Both Diffie-Hellman results feed HKDF-SHA256, salted with
//! the two ephemeral keys and bound to both device ids and to the hash of the
//! exact hello texts both sides sent, giving one AES-256-GCM key per
//! direction. A hello altered in transit (capabilities, names, protocol)
//! leaves the two sides with different keys, so the encrypted ready frame
//! fails. Frames carry a per-direction counter; a frame whose
//! counter is not the next expected one is refused, so frames cannot be
//! replayed, dropped, or reordered unnoticed.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

use super::identity::{clean_name, device_id_of, ShareIdentity};

const KDF_INFO: &[u8] = b"OpenBitFun LAN share v1";
const COUNTER_BYTES: usize = 8;

/// The plaintext opening message of each side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol: u32,
    pub capabilities: Vec<String>,
    pub device_id: String,
    pub device_name: String,
    pub display_name: String,
    pub public_key: String,
    pub ephemeral_key: String,
}

/// A hello whose fields were checked.
#[derive(Debug, Clone)]
pub struct CheckedHello {
    pub protocol: u32,
    pub capabilities: Vec<String>,
    pub device_id: String,
    pub device_name: String,
    pub display_name: String,
    pub public_key: [u8; 32],
    pub ephemeral_key: [u8; 32],
}

impl Hello {
    pub fn check(self) -> Result<CheckedHello, String> {
        let key = |value: &str, what: &str| -> Result<[u8; 32], String> {
            BASE64
                .decode(value)
                .ok()
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .ok_or_else(|| format!("the peer sent an invalid {what}"))
        };
        let public_key = key(&self.public_key, "public key")?;
        if self.device_id != device_id_of(&public_key) {
            return Err("the peer's device id does not belong to its key".to_string());
        }
        if self.capabilities.len() > 64 || self.capabilities.iter().any(|c| c.len() > 64) {
            return Err("the peer sent an invalid capability list".to_string());
        }
        Ok(CheckedHello {
            protocol: self.protocol,
            capabilities: self.capabilities,
            device_id: self.device_id,
            device_name: clean_name(&self.device_name)
                .ok_or_else(|| "the peer sent an empty device name".to_string())?,
            display_name: clean_name(&self.display_name)
                .ok_or_else(|| "the peer sent an empty display name".to_string())?,
            public_key,
            ephemeral_key: key(&self.ephemeral_key, "ephemeral key")?,
        })
    }
}

/// This side's half of a handshake in progress.
pub struct LocalHandshake {
    ephemeral: EphemeralSecret,
    ephemeral_public: [u8; 32],
}

impl LocalHandshake {
    pub fn new() -> Self {
        let ephemeral = EphemeralSecret::random_from_rng(rand::rngs::OsRng);
        let ephemeral_public = PublicKey::from(&ephemeral).to_bytes();
        Self {
            ephemeral,
            ephemeral_public,
        }
    }

    pub fn hello(&self, identity: &ShareIdentity, protocol: u32, capabilities: &[String]) -> Hello {
        Hello {
            protocol,
            capabilities: capabilities.to_vec(),
            device_id: identity.device_id().to_string(),
            device_name: identity.device_name().to_string(),
            display_name: identity.display_name().to_string(),
            public_key: BASE64.encode(identity.public_key()),
            ephemeral_key: BASE64.encode(self.ephemeral_public),
        }
    }

    /// Derives the two direction keys. `initiator` is true on the side that
    /// opened the connection. `local_hello` and `peer_hello` are the exact
    /// hello texts sent and received, which the keys are bound to.
    pub fn finish(
        self,
        identity: &ShareIdentity,
        peer: &CheckedHello,
        initiator: bool,
        local_hello: &str,
        peer_hello: &str,
    ) -> Result<SessionKeys, String> {
        let static_shared = StaticSecret::from(*identity.secret())
            .diffie_hellman(&PublicKey::from(peer.public_key));
        let ephemeral_shared = self
            .ephemeral
            .diffie_hellman(&PublicKey::from(peer.ephemeral_key));
        if !static_shared.was_contributory() || !ephemeral_shared.was_contributory() {
            return Err("the peer sent a weak key".to_string());
        }
        if peer.device_id == identity.device_id() {
            return Err("the peer uses this device's own identity".to_string());
        }
        let (initiator_ephemeral, responder_ephemeral, initiator_id, responder_id) = if initiator {
            (
                self.ephemeral_public,
                peer.ephemeral_key,
                identity.device_id(),
                peer.device_id.as_str(),
            )
        } else {
            (
                peer.ephemeral_key,
                self.ephemeral_public,
                peer.device_id.as_str(),
                identity.device_id(),
            )
        };
        let (initiator_hello, responder_hello) = if initiator {
            (local_hello, peer_hello)
        } else {
            (peer_hello, local_hello)
        };
        let mut salt = [0u8; 64];
        salt[..32].copy_from_slice(&initiator_ephemeral);
        salt[32..].copy_from_slice(&responder_ephemeral);
        let mut ikm = [0u8; 64];
        ikm[..32].copy_from_slice(static_shared.as_bytes());
        ikm[32..].copy_from_slice(ephemeral_shared.as_bytes());
        let info = [
            KDF_INFO,
            initiator_id.as_bytes(),
            responder_id.as_bytes(),
            &transcript_hash(initiator_hello, responder_hello),
        ]
        .concat();
        let mut okm = [0u8; 64];
        Hkdf::<Sha256>::new(Some(&salt), &ikm)
            .expand(&info, &mut okm)
            .map_err(|_| "key derivation failed".to_string())?;
        let mut forward = [0u8; 32];
        let mut backward = [0u8; 32];
        forward.copy_from_slice(&okm[..32]);
        backward.copy_from_slice(&okm[32..]);
        let (send, receive, send_direction) = if initiator {
            (forward, backward, 0)
        } else {
            (backward, forward, 1)
        };
        Ok(SessionKeys {
            sender: FrameSealer::new(&send, send_direction),
            receiver: FrameOpener::new(&receive, 1 - send_direction),
        })
    }
}

/// SHA-256 over both hello texts, each prefixed by its length so the
/// boundary between them is unambiguous.
fn transcript_hash(initiator_hello: &str, responder_hello: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for hello in [initiator_hello, responder_hello] {
        hasher.update((hello.len() as u64).to_be_bytes());
        hasher.update(hello.as_bytes());
    }
    hasher.finalize().into()
}

impl Default for LocalHandshake {
    fn default() -> Self {
        Self::new()
    }
}

/// The two halves of an established session.
pub struct SessionKeys {
    pub sender: FrameSealer,
    pub receiver: FrameOpener,
}

/// Encrypts outgoing frames in order.
pub struct FrameSealer {
    cipher: Aes256Gcm,
    direction: u8,
    counter: u64,
}

impl FrameSealer {
    pub fn new(key: &[u8; 32], direction: u8) -> Self {
        Self {
            cipher: Aes256Gcm::new(key.into()),
            direction,
            counter: 0,
        }
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let counter = self.counter;
        self.counter = counter
            .checked_add(1)
            .ok_or_else(|| "the frame counter is exhausted".to_string())?;
        let ciphertext = self
            .cipher
            .encrypt(&nonce(self.direction, counter), plaintext)
            .map_err(|_| "frame encryption failed".to_string())?;
        let mut frame = Vec::with_capacity(COUNTER_BYTES + ciphertext.len());
        frame.extend_from_slice(&counter.to_be_bytes());
        frame.extend_from_slice(&ciphertext);
        Ok(frame)
    }
}

/// Decrypts incoming frames and refuses any out of order.
pub struct FrameOpener {
    cipher: Aes256Gcm,
    direction: u8,
    expected: u64,
}

impl FrameOpener {
    pub fn new(key: &[u8; 32], direction: u8) -> Self {
        Self {
            cipher: Aes256Gcm::new(key.into()),
            direction,
            expected: 0,
        }
    }

    pub fn open(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.len() < COUNTER_BYTES {
            return Err("the peer sent a truncated frame".to_string());
        }
        let counter = u64::from_be_bytes(frame[..COUNTER_BYTES].try_into().expect("8 bytes"));
        if counter != self.expected {
            return Err("the peer sent a replayed or out-of-order frame".to_string());
        }
        let plaintext = self
            .cipher
            .decrypt(&nonce(self.direction, counter), &frame[COUNTER_BYTES..])
            .map_err(|_| "the peer sent a frame that failed authentication".to_string())?;
        self.expected += 1;
        Ok(plaintext)
    }
}

fn nonce(direction: u8, counter: u64) -> Nonce<aes_gcm::aead::consts::U12> {
    let mut bytes = [0u8; 12];
    bytes[0] = direction;
    bytes[4..].copy_from_slice(&counter.to_be_bytes());
    *Nonce::from_slice(&bytes)
}
