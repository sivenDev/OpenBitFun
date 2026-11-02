//! Target-side admission of delegated dispatch jobs.
//!
//! A delegated job names one of this device's opened assistants. Every task
//! is a message task that runs as the assistant in a directory of its own,
//! which this device chooses; the requesting device is the host-bound sender
//! of the device RPC.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use openbitfun_core::service::delegation::{
    assistant_directory, delegation_workspaces_root, OpenAgentStore,
};
use sha2::{Digest, Sha256};

use super::protocol::{DispatchApprovalPolicy, DispatchSubmitRequest};
use super::store::create_private_dir;

const MAX_SOURCE_FIELD_CHARS: usize = 256;

/// Where admitted jobs may run on this device.
pub(super) struct DelegationRoots {
    /// Parent of the per-task directories of delegated tasks.
    pub(super) message_workspaces: PathBuf,
    /// Parent of this device's assistant workspaces.
    pub(super) assistants: PathBuf,
}

impl DelegationRoots {
    pub(super) fn for_device() -> Result<Self> {
        let path_manager = openbitfun_core::infrastructure::PathManager::new()
            .map_err(|error| anyhow::anyhow!("resolve OpenBitFun storage root: {error}"))?;
        Ok(Self {
            message_workspaces: delegation_workspaces_root(&path_manager),
            assistants: path_manager.assistant_workspace_base_dir(None),
        })
    }
}

/// The device-local open-agent list.
pub(super) fn open_agent_store() -> Result<OpenAgentStore> {
    let path_manager = openbitfun_core::infrastructure::PathManager::new()
        .map_err(|error| anyhow::anyhow!("resolve OpenBitFun storage root: {error}"))?;
    Ok(OpenAgentStore::for_product_home(&path_manager))
}

/// Admits `request` before a job is created. Ordinary jobs pass unchanged; a
/// delegated job is checked against the opened assistant and gets its task
/// directory and the assistant's workspace from this device.
pub(super) async fn admit(
    request: &mut DispatchSubmitRequest,
    store: &OpenAgentStore,
    roots: &DelegationRoots,
) -> Result<()> {
    let Some(delegation) = request.delegation.clone() else {
        if request.source_device_id.is_some() {
            bail!("__sourceDeviceId is reserved for delegated dispatch jobs");
        }
        if request.source_connection_id.is_some() {
            bail!("__sourceConnectionId is reserved for delegated dispatch jobs");
        }
        if request.assistant_workspace.is_some() {
            bail!("__assistantWorkspace is chosen by this device");
        }
        return Ok(());
    };
    if request.assistant_workspace.is_some() {
        bail!("__assistantWorkspace is chosen by this device");
    }
    let requester = match (
        request
            .source_device_id
            .clone()
            .filter(|device_id| !device_id.trim().is_empty()),
        request
            .source_connection_id
            .clone()
            .filter(|connection_id| !connection_id.trim().is_empty()),
    ) {
        (Some(device_id), None) => Requester::Device(device_id),
        (None, Some(connection_id)) => Requester::Connection(connection_id),
        _ => bail!(
            "delegated dispatch jobs are accepted only from another device of this account or a paired connection"
        ),
    };
    let source = &delegation.source;
    for value in [&source.session_id, &source.agent_type, &source.display_name]
        .into_iter()
        .flatten()
    {
        if value.chars().count() > MAX_SOURCE_FIELD_CHARS {
            bail!("delegation source fields are limited to {MAX_SOURCE_FIELD_CHARS} characters");
        }
    }
    if request.approval_policy != DispatchApprovalPolicy::Remote {
        bail!("delegated dispatch jobs require the remote approval policy");
    }
    let agent = match &requester {
        Requester::Device(device_id) => store.authorize(device_id, &delegation.open_agent_id).await,
        Requester::Connection(connection_id) => {
            store
                .authorize_connection(connection_id, &delegation.open_agent_id)
                .await
        }
    }
    .context("read agents opened to other devices")?
    .map_err(anyhow::Error::new)?;
    if matches!(requester, Requester::Connection(_)) {
        // This device chooses the model of jobs another account delegates.
        request.model = None;
        request.reasoning_preset = None;
    }
    if request.agent_type != agent.agent_type {
        bail!(
            "open agent '{}' runs agent '{}', not '{}'",
            agent.id,
            agent.agent_type,
            request.agent_type
        );
    }

    let Some(directory) = assistant_directory(&agent.agent_type) else {
        bail!("open agent '{}' is not an assistant", agent.id);
    };
    // An opened assistant answers each task in a directory of its own, and
    // reads its persona from its workspace without writing there.
    if !request.workspace_path.trim().is_empty() {
        bail!("assistant tasks run in a directory chosen by this device; omit workspacePath");
    }
    let assistant_root = roots.assistants.join(directory);
    if !assistant_root.is_dir() {
        bail!(
            "open agent '{}' names an assistant that no longer exists on this device",
            agent.id
        );
    }
    let dir = roots
        .message_workspaces
        .join(&agent.id)
        .join(requester.directory_name())
        .join(device_directory_name(&request.job_id));
    create_private_dir(&dir)?;
    request.workspace_path = dir.to_string_lossy().into_owned();
    request.assistant_workspace = Some(assistant_root.to_string_lossy().into_owned());
    Ok(())
}

/// Who delegated a job: a device of this account or a cross-account
/// connection. Each has its own grants and its own task directories.
enum Requester {
    Device(String),
    Connection(String),
}

impl Requester {
    fn directory_name(&self) -> String {
        match self {
            Self::Device(device_id) => device_directory_name(device_id),
            // Device ids never start with "connection-".
            Self::Connection(connection_id) => {
                format!("connection-{}", device_directory_name(connection_id))
            }
        }
    }
}

/// A directory name for the requester that cannot traverse or collide with
/// another requester's directory.
fn device_directory_name(device_id: &str) -> String {
    let safe = !device_id.is_empty()
        && device_id.len() <= 64
        && device_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if safe {
        device_id.to_string()
    } else {
        Sha256::digest(device_id.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// Opens assistants for tests of the dispatch target.
#[cfg(test)]
pub(super) mod test_support {
    use openbitfun_core::service::delegation::{OpenAgent, OpenAgentStore};
    use std::path::Path;

    /// Opens the assistant `directory` under `assistants` as `id`, creating
    /// its directory, to the same-account `devices`.
    pub(crate) async fn open_assistant(
        store: &OpenAgentStore,
        assistants: &Path,
        id: &str,
        directory: &str,
        devices: &[&str],
    ) {
        std::fs::create_dir_all(assistants.join(directory)).unwrap();
        store
            .open_assistant(
                assistants,
                OpenAgent {
                    id: id.to_string(),
                    agent_type: format!("assistant:{directory}"),
                    display_name: id.to_string(),
                    description: None,
                    same_account_devices: devices.iter().map(|d| d.to_string()).collect(),
                    connections: Vec::new(),
                },
                true,
            )
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::protocol::{DispatchDelegation, DispatchDelegationSource};
    use std::path::Path;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: OpenAgentStore,
        roots: DelegationRoots,
        elsewhere: PathBuf,
    }

    const HELPER_TYPE: &str = "assistant:workspace";

    async fn open_assistant(fixture: &Fixture, id: &str, directory: &str) {
        super::test_support::open_assistant(
            &fixture.store,
            &fixture.roots.assistants,
            id,
            directory,
            &["laptop"],
        )
        .await;
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let elsewhere = dir.path().join("notes");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let fixture = Fixture {
            store: OpenAgentStore::new(dir.path().join("open-agents.json")),
            roots: DelegationRoots {
                message_workspaces: dir.path().join("message-workspaces"),
                assistants: dir.path().join("personal_assistant"),
            },
            elsewhere,
            _dir: dir,
        };
        open_assistant(&fixture, "helper", "workspace").await;
        fixture
    }

    fn delegated(open_agent_id: &str) -> DispatchSubmitRequest {
        DispatchSubmitRequest {
            protocol_version:
                openbitfun_services_core::dispatch_contract::DISPATCH_PROTOCOL_VERSION,
            job_id: "job-1".to_string(),
            session_id: "session-1".to_string(),
            workspace_path: String::new(),
            agent_type: HELPER_TYPE.to_string(),
            prompt: "What changed?".to_string(),
            approval_policy: DispatchApprovalPolicy::Remote,
            model: None,
            reasoning_preset: None,
            title: None,
            attachments: Vec::new(),
            setup_audit: Vec::new(),
            delegation: Some(DispatchDelegation {
                open_agent_id: open_agent_id.to_string(),
                source: DispatchDelegationSource::default(),
            }),
            source_device_id: Some("laptop".to_string()),
            source_connection_id: None,
            assistant_workspace: None,
        }
    }

    async fn admit_error(fixture: &Fixture, mut request: DispatchSubmitRequest) -> String {
        format!(
            "{:#}",
            admit(&mut request, &fixture.store, &fixture.roots)
                .await
                .expect_err("refused")
        )
    }

    #[tokio::test]
    async fn assistant_tasks_get_a_directory_each_and_the_assistant_this_device_holds() {
        let fixture = fixture().await;
        open_assistant(&fixture, "ann", "workspace-ann").await;
        let assistant_root = fixture.roots.assistants.join("workspace-ann");
        let ask = |job_id: &str| {
            let mut request = delegated("ann");
            request.job_id = job_id.to_string();
            request.agent_type = "assistant:workspace-ann".to_string();
            request
        };

        let mut first = ask("job-1");
        admit(&mut first, &fixture.store, &fixture.roots)
            .await
            .unwrap();
        let mut second = ask("job-2");
        admit(&mut second, &fixture.store, &fixture.roots)
            .await
            .unwrap();
        let laptop = fixture.roots.message_workspaces.join("ann").join("laptop");
        assert_eq!(first.workspace_path, laptop.join("job-1").to_string_lossy());
        assert_eq!(
            second.workspace_path,
            laptop.join("job-2").to_string_lossy()
        );
        assert!(Path::new(&first.workspace_path).is_dir());
        assert_eq!(
            first.assistant_workspace.as_deref(),
            Some(assistant_root.to_string_lossy().as_ref())
        );
        // Admitting the same job again chooses the same directory.
        let mut retried = ask("job-1");
        admit(&mut retried, &fixture.store, &fixture.roots)
            .await
            .unwrap();
        assert_eq!(retried, first);

        // The requester never chooses the directory or the assistant.
        let mut chosen = ask("job-3");
        chosen.workspace_path = fixture.elsewhere.to_string_lossy().into_owned();
        assert!(admit_error(&fixture, chosen)
            .await
            .contains("omit workspacePath"));
        let mut injected = ask("job-3");
        injected.assistant_workspace = Some("/".to_string());
        assert!(admit_error(&fixture, injected)
            .await
            .contains("chosen by this device"));

        // An assistant removed after it was opened refuses new tasks.
        std::fs::remove_dir_all(&assistant_root).unwrap();
        assert!(admit_error(&fixture, ask("job-4"))
            .await
            .contains("no longer exists"));
    }

    #[tokio::test]
    async fn connections_use_their_own_grants_directory_and_no_model_choice() {
        let fixture = fixture().await;
        let over = |connection: &str| {
            let mut request = delegated("helper");
            request.source_device_id = None;
            request.source_connection_id = Some(connection.to_string());
            request.model = Some("big-model".to_string());
            request.reasoning_preset = Some("high".to_string());
            request
        };

        // A grant to the device "laptop" does not admit a connection.
        assert!(admit_error(&fixture, over("conn-1"))
            .await
            .contains("not granted to this connection"));

        fixture
            .store
            .grant_connection("helper", "conn-1")
            .await
            .unwrap();
        let mut request = over("conn-1");
        admit(&mut request, &fixture.store, &fixture.roots)
            .await
            .unwrap();
        let expected = fixture
            .roots
            .message_workspaces
            .join("helper")
            .join("connection-conn-1")
            .join("job-1");
        assert_eq!(request.workspace_path, expected.to_string_lossy());
        assert!(expected.is_dir());
        assert_eq!(request.model, None);
        assert_eq!(request.reasoning_preset, None);

        assert!(admit_error(&fixture, over("conn-2"))
            .await
            .contains("not granted to this connection"));

        // A request naming both a device and a connection is refused.
        let mut both = over("conn-1");
        both.source_device_id = Some("laptop".to_string());
        assert!(admit_error(&fixture, both)
            .await
            .contains("accepted only from"));

        // An ordinary job cannot claim a connection.
        let mut ordinary = over("conn-1");
        ordinary.delegation = None;
        assert!(admit_error(&fixture, ordinary)
            .await
            .contains("__sourceConnectionId is reserved"));
    }

    #[tokio::test]
    async fn refusals_name_the_reason_and_create_nothing() {
        let fixture = fixture().await;

        let mut stranger = delegated("helper");
        stranger.source_device_id = Some("phone".to_string());
        assert!(admit_error(&fixture, stranger)
            .await
            .contains("not granted"));

        let missing = delegated("nobody");
        assert!(admit_error(&fixture, missing)
            .await
            .contains("does not exist"));

        let mut wrong_agent = delegated("helper");
        wrong_agent.agent_type = "Standard".to_string();
        assert!(admit_error(&fixture, wrong_agent)
            .await
            .contains("runs agent 'assistant:workspace'"));

        let mut unattended = delegated("helper");
        unattended.approval_policy = DispatchApprovalPolicy::Auto;
        assert!(admit_error(&fixture, unattended)
            .await
            .contains("remote approval policy"));

        let mut anonymous = delegated("helper");
        anonymous.source_device_id = None;
        assert!(admit_error(&fixture, anonymous)
            .await
            .contains("another device"));

        // None of the refusals created a task directory.
        assert!(!fixture.roots.message_workspaces.exists());
    }

    #[tokio::test]
    async fn ordinary_jobs_cannot_claim_a_source_device() {
        let fixture = fixture().await;
        let mut ordinary = delegated("helper");
        ordinary.delegation = None;
        ordinary.workspace_path = "/repo".to_string();
        assert!(admit_error(&fixture, ordinary.clone())
            .await
            .contains("reserved for delegated"));
        ordinary.source_device_id = None;
        admit(&mut ordinary, &fixture.store, &fixture.roots)
            .await
            .unwrap();
        assert_eq!(ordinary.workspace_path, "/repo");
    }

    #[test]
    fn requester_directories_never_traverse() {
        assert_eq!(device_directory_name("laptop-1"), "laptop-1");
        for hostile in ["../escape", "a/b", "", ".."] {
            let name = device_directory_name(hostile);
            assert_eq!(name.len(), 64, "{hostile}");
            assert!(name.chars().all(|ch| ch.is_ascii_hexdigit()));
        }
    }
}
