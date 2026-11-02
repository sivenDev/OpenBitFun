//! Tenant workspaces: one per device that uses a shared assistant.
//!
//! The assistant's own workspace belongs to its owner. Each device the owner
//! shares the assistant with gets its own workspace under
//! `<product home>/delegation/tenants/<assistant id>/<device id>/`, outside
//! the assistant workspace, so the assistant never sees a tenant's files in
//! the owner's conversations and tenants never see each other. Every task of
//! that device runs there and its sessions are stored for that directory. The
//! path follows from the assistant and the device alone; no caller supplies it.
//!
//! A tenant workspace holds the user's own `USER.md`. The assistant's identity
//! and persona are read from the assistant workspace, and a tenant workspace
//! never gets `BOOTSTRAP.md`.

use std::path::{Path, PathBuf};

use log::{info, warn};

use super::shared_assistants::is_share_device_id;
#[cfg(feature = "local-storage")]
use super::shared_assistants::{SharedAssistant, SharedAssistantStore};
use crate::infrastructure::PathManager;
use crate::service::delegation::{assistant_agent_type, assistant_directory};

const DELEGATION_DIR: &str = "delegation";
const TENANTS_DIR: &str = "tenants";
const USER_FILE_NAME: &str = "USER.md";

/// Parent of every tenant workspace. Nothing under it is a source of the
/// owner's memories.
pub fn tenants_root(path_manager: &PathManager) -> PathBuf {
    path_manager
        .product_home_dir()
        .join(DELEGATION_DIR)
        .join(TENANTS_DIR)
}

/// Whether `path` lies in a tenant workspace.
pub fn is_tenant_path(path_manager: &PathManager, path: &Path) -> bool {
    path.starts_with(tenants_root(path_manager))
}

/// The tenant workspace of `device_id` under the shared assistant
/// `assistant_id`, within `root`.
pub fn tenant_dir(root: &Path, assistant_id: &str, device_id: &str) -> Result<PathBuf, String> {
    if assistant_directory(&assistant_agent_type(assistant_id)) != Some(assistant_id) {
        return Err(format!("'{assistant_id}' does not name an assistant"));
    }
    if !is_share_device_id(device_id) {
        return Err("a tenant is named by a device id derived from its sharing key".to_string());
    }
    Ok(root.join(assistant_id).join(device_id))
}

/// Who a tenant workspace belongs to, as its `USER.md` starts out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantUser {
    pub display_name: String,
    pub device_name: String,
}

/// Creates the tenant workspace if it does not exist yet, with a `USER.md`
/// naming its user, and returns it. An existing `USER.md` is never replaced:
/// the assistant fills it in over the tenant's conversations.
pub fn ensure_tenant(
    root: &Path,
    assistant_id: &str,
    device_id: &str,
    user: &TenantUser,
) -> std::io::Result<PathBuf> {
    let dir = tenant_dir(root, assistant_id, device_id)
        .map_err(|problem| std::io::Error::new(std::io::ErrorKind::InvalidInput, problem))?;
    for directory in [root.to_path_buf(), root.join(assistant_id), dir.clone()] {
        create_private_dir(&directory)?;
    }
    let user_file = dir.join(USER_FILE_NAME);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&user_file)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(tenant_user_profile(user).as_bytes())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    Ok(dir)
}

fn tenant_user_profile(user: &TenantUser) -> String {
    format!(
        "# USER.md - About the Person You're Helping\n\n\
         _This person uses you from another device on the local network. Learn about them and update this as you go. It is theirs alone: your owner and other users never see it._\n\n\
         - **Name:** {}\n\
         - **Device:** {}\n\
         - **What to call them:**\n\
         - **Notes:**\n",
        user.display_name.trim(),
        user.device_name.trim()
    )
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(error) => Err(error),
    }
}

/// One tenant workspace on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    pub assistant_id: String,
    pub device_id: String,
    pub path: PathBuf,
}

/// Every tenant workspace under `root`. Entries that are not tenant
/// workspaces, such as links, are skipped.
pub fn list_tenants(root: &Path) -> Vec<Tenant> {
    let mut tenants = Vec::new();
    for assistant in read_dirs(root) {
        let assistant_id = assistant.file_name().to_string_lossy().into_owned();
        for device in read_dirs(&assistant.path()) {
            let device_id = device.file_name().to_string_lossy().into_owned();
            if tenant_dir(root, &assistant_id, &device_id).is_ok() {
                tenants.push(Tenant {
                    assistant_id: assistant_id.clone(),
                    device_id,
                    path: device.path(),
                });
            }
        }
    }
    tenants.sort_by(|left, right| left.path.cmp(&right.path));
    tenants
}

fn read_dirs(path: &Path) -> Vec<std::fs::DirEntry> {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .collect()
}

/// Deletes the tenant workspace of `device_id` under `assistant_id` with its
/// sessions and runtime data. Returns whether it existed.
pub fn delete_tenant(
    path_manager: &PathManager,
    assistant_id: &str,
    device_id: &str,
) -> std::io::Result<bool> {
    let dir = tenant_dir(&tenants_root(path_manager), assistant_id, device_id)
        .map_err(|problem| std::io::Error::new(std::io::ErrorKind::InvalidInput, problem))?;
    let existed = remove_dir_if_real(&path_manager.project_runtime_root(&dir))?;
    let existed = remove_dir_if_real(&dir)? || existed;
    if existed {
        info!(
            "Tenant workspace deleted: assistant_id={}, device_id={}",
            assistant_id, device_id
        );
    }
    Ok(existed)
}

fn remove_dir_if_real(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            std::fs::remove_dir_all(path)?;
            Ok(true)
        }
        Ok(_) => {
            warn!(
                "Skipping a tenant path that is not a directory: {}",
                path.display()
            );
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(feature = "local-storage")]
/// Deletes the tenant workspaces whose device no longer holds a grant and
/// that have no sessions left. Returns the ones it deleted. When the shares
/// cannot be read, nothing is deleted; a workspace whose sessions cannot be
/// read is kept.
pub async fn prune_tenants(
    path_manager: &PathManager,
    store: &SharedAssistantStore,
) -> Result<Vec<Tenant>, String> {
    let shares = store
        .shares()
        .await
        .map_err(|error| format!("read shared assistants: {error}"))?;
    let mut deleted = Vec::new();
    for tenant in list_tenants(&tenants_root(path_manager)) {
        if is_granted(&shares, &tenant) {
            continue;
        }
        let sessions_dir = path_manager.project_sessions_dir(&tenant.path);
        let sessions = openbitfun_services_core::session::SessionMetadataStore::new(&sessions_dir);
        let listed = if sessions_dir.exists() {
            sessions.list_metadata_including_internal().await
        } else {
            Ok(Vec::new())
        };
        match listed {
            Ok(list) if list.is_empty() => {}
            Ok(_) => continue,
            Err(error) => {
                warn!(
                    "Keeping tenant workspace, sessions unreadable: path={}, error={}",
                    tenant.path.display(),
                    error
                );
                continue;
            }
        }
        match delete_tenant(path_manager, &tenant.assistant_id, &tenant.device_id) {
            Ok(_) => deleted.push(tenant),
            Err(error) => warn!(
                "Failed to delete tenant workspace: path={}, error={}",
                tenant.path.display(),
                error
            ),
        }
    }
    Ok(deleted)
}

#[cfg(feature = "local-storage")]
fn is_granted(shares: &[SharedAssistant], tenant: &Tenant) -> bool {
    shares
        .iter()
        .find(|share| share.id == tenant.assistant_id)
        .is_some_and(|share| share.is_granted_to_device_id(&tenant.device_id))
}
