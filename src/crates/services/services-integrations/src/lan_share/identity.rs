//! The device's sharing identity: an X25519 key pair kept on this device,
//! independent of any account, plus the names peers see.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use super::MAX_NAME_CHARS;

const IDENTITY_FORMAT: u32 = 1;
const FINGERPRINT_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const FINGERPRINT_CHARS: usize = 6;

/// A device's sharing identity.
#[derive(Clone)]
pub struct ShareIdentity {
    device_id: String,
    device_name: String,
    display_name: String,
    secret: [u8; 32],
}

impl std::fmt::Debug for ShareIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShareIdentity")
            .field("device_id", &self.device_id)
            .field("device_name", &self.device_name)
            .field("display_name", &self.display_name)
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredIdentity {
    format: u32,
    display_name: String,
    secret: String,
}

impl ShareIdentity {
    /// A new identity with a fresh key pair. The device id follows from the
    /// public key, so no device can claim another's id with its own key.
    pub fn generate(device_name: &str, display_name: &str) -> Result<Self> {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        Ok(Self {
            device_id: device_id_of(&PublicKey::from(&secret).to_bytes()),
            device_name: clean_name(device_name).ok_or_else(|| anyhow!("device name is empty"))?,
            display_name: clean_name(display_name)
                .ok_or_else(|| anyhow!("display name is empty"))?,
            secret: secret.to_bytes(),
        })
    }

    /// Where this device keeps its sharing identity.
    pub fn default_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("home directory is unavailable"))?;
        Ok(home
            .join(".openbitfun")
            .join("lan-share")
            .join("identity.json"))
    }

    /// Loads the identity at `path`, creating and saving one when the file
    /// does not exist. An unreadable file is reported, never replaced.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Self::decode(&bytes).with_context(|| {
                format!(
                    "the LAN sharing identity at {} is unreadable; move it away to create a new one",
                    path.display()
                )
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let identity = Self::generate(&local_device_name(), &local_display_name())?;
                identity.save(path)?;
                Ok(identity)
            }
            Err(error) => Err(error)
                .with_context(|| format!("read the LAN sharing identity at {}", path.display())),
        }
    }

    /// Loads the identity at `path`, for a process that acts as this device
    /// but must never create one: a missing file is an error.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("read the LAN sharing identity at {}", path.display()))?;
        Self::decode(&bytes).with_context(|| {
            format!(
                "the LAN sharing identity at {} is unreadable",
                path.display()
            )
        })
    }

    /// Writes the identity with owner-only permissions, atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        let stored = StoredIdentity {
            format: IDENTITY_FORMAT,
            display_name: self.display_name.clone(),
            secret: BASE64.encode(self.secret),
        };
        let bytes =
            serde_json::to_vec_pretty(&stored).context("encode the LAN sharing identity")?;
        crate::remote_persistence::write_private_bytes(path, &bytes)
            .with_context(|| format!("write the LAN sharing identity at {}", path.display()))
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let stored: StoredIdentity = serde_json::from_slice(bytes)?;
        if stored.format != IDENTITY_FORMAT {
            bail!("unsupported identity format {}", stored.format);
        }
        let secret: [u8; 32] = BASE64
            .decode(stored.secret)?
            .try_into()
            .map_err(|_| anyhow!("invalid key length"))?;
        Ok(Self {
            device_id: device_id_of(&PublicKey::from(&StaticSecret::from(secret)).to_bytes()),
            device_name: local_device_name(),
            display_name: clean_name(&stored.display_name)
                .ok_or_else(|| anyhow!("display name is empty"))?,
            secret,
        })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Changes the name peers see. Call [`Self::save`] to keep it.
    pub fn set_display_name(&mut self, name: &str) -> Result<()> {
        self.display_name = clean_name(name).ok_or_else(|| anyhow!("display name is empty"))?;
        Ok(())
    }

    pub fn public_key(&self) -> [u8; 32] {
        PublicKey::from(&StaticSecret::from(self.secret)).to_bytes()
    }

    pub fn fingerprint(&self) -> String {
        fingerprint_of(&self.public_key())
    }

    pub(crate) fn secret(&self) -> &[u8; 32] {
        &self.secret
    }
}

/// The short code people compare to tell devices apart: six characters from
/// the SHA-256 of the public key, without easily confused letters or digits.
/// It is a display aid only; connections pin the full public key.
pub fn fingerprint_of(public_key: &[u8; 32]) -> String {
    let digest = Sha256::digest(public_key);
    let mut bits = u64::from_be_bytes(digest[..8].try_into().expect("8 bytes"));
    let mut code = String::with_capacity(FINGERPRINT_CHARS);
    for _ in 0..FINGERPRINT_CHARS {
        code.push(FINGERPRINT_ALPHABET[(bits >> 59) as usize] as char);
        bits <<= 5;
    }
    code
}

/// The device id that belongs to a sharing key: 32 hex characters of the
/// SHA-256 of the public key.
pub fn device_id_of(public_key: &[u8; 32]) -> String {
    hex(&Sha256::digest(public_key)[16..])
}

/// Whether `value` has the shape of a fingerprint code.
pub fn is_fingerprint(value: &str) -> bool {
    value.len() == FINGERPRINT_CHARS && value.bytes().all(|b| FINGERPRINT_ALPHABET.contains(&b))
}

/// Trims a name and bounds its length; `None` when nothing is left.
pub(crate) fn clean_name(value: &str) -> Option<String> {
    let trimmed: String = value
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn local_device_name() -> String {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .map(|name| name.trim_end_matches(".local").to_string())
        .and_then(|name| clean_name(&name))
        .unwrap_or_else(|| "OpenBitFun".to_string())
}

/// The account's full name where the system keeps one, else its login name.
fn local_display_name() -> String {
    account_full_name()
        .or_else(|| {
            ["USER", "USERNAME"]
                .iter()
                .find_map(|key| std::env::var(key).ok())
                .and_then(|name| clean_name(&name))
        })
        .unwrap_or_else(local_device_name)
}

#[cfg(unix)]
fn account_full_name() -> Option<String> {
    // SAFETY: `passwd` is a plain C struct of integers and raw pointers, for
    // which all-zero bytes are a valid value; `getpwuid_r` fills it in.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer = vec![0 as libc::c_char; 4096];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer refers to storage owned by this frame, and the
    // returned strings are read before `buffer` is dropped.
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() || entry.pw_gecos.is_null() {
        return None;
    }
    // SAFETY: `pw_gecos` is a NUL-terminated string inside `buffer`.
    let gecos = unsafe { std::ffi::CStr::from_ptr(entry.pw_gecos) };
    let gecos = gecos.to_string_lossy();
    clean_name(gecos.split(',').next().unwrap_or_default())
}

#[cfg(not(unix))]
fn account_full_name() -> Option<String> {
    None
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
