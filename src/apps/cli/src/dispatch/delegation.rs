//! Target-side admission of delegated dispatch jobs.
//!
//! A delegated job names one of this device's open agents. The open agent, not
//! the requester, decides the agent, the allowed task kinds, and where message
//! tasks run; the requesting device is the host-bound sender of the device RPC.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use openbitfun_core::service::delegation::{
    assistant_directory, delegation_workspaces_root, DelegationTaskKind, OpenAgentStore,
};
use sha2::{Digest, Sha256};

use super::protocol::{DispatchApprovalPolicy, DispatchDelegationTaskKind, DispatchSubmitRequest};
use super::store::create_private_dir;

const MAX_SOURCE_FIELD_CHARS: usize = 256;

/// Where admitted jobs may run on this device.
pub(super) struct DelegationRoots {
    /// Parent of the per-requester directories for unbound message tasks.
    pub(super) message_workspaces: PathBuf,
    /// Parent of the Git worktrees this target provisions for coding tasks.
    pub(super) worktrees: PathBuf,
    /// Parent of this device's assistant workspaces.
    pub(super) assistants: PathBuf,
}

impl DelegationRoots {
    pub(super) fn for_store(store: &super::store::DispatchStore) -> Result<Self> {
        let path_manager = openbitfun_core::infrastructure::PathManager::new()
            .map_err(|error| anyhow::anyhow!("resolve OpenBitFun storage root: {error}"))?;
        Ok(Self {
            message_workspaces: delegation_workspaces_root(&path_manager),
            worktrees: store.worktrees_root(),
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
/// delegated job is checked against the open agent and gets its workspace
/// path from this device.
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
    let kind = match delegation.task_kind {
        DispatchDelegationTaskKind::Message => DelegationTaskKind::Message,
        DispatchDelegationTaskKind::Coding => DelegationTaskKind::Coding,
    };
    let agent = match &requester {
        Requester::Device(device_id) => {
            store
                .authorize(device_id, &delegation.open_agent_id, kind)
                .await
        }
        Requester::Connection(connection_id) => {
            store
                .authorize_connection(connection_id, &delegation.open_agent_id, kind)
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

    if let Some(directory) = assistant_directory(&agent.agent_type) {
        // An opened assistant answers each task in a directory of its own,
        // and reads its persona from its workspace without writing there.
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
        return Ok(());
    }

    request.workspace_path = match delegation.task_kind {
        DispatchDelegationTaskKind::Message => {
            if !request.workspace_path.trim().is_empty() {
                bail!("message tasks run in a directory chosen by this device; omit workspacePath");
            }
            // Another account's tasks never share a directory, or its
            // sessions, with anyone else: a bound directory serves only this
            // account's devices.
            let bound = match &requester {
                Requester::Device(_) => agent.message_workspace.as_ref(),
                Requester::Connection(_) => None,
            };
            let workspace = match bound {
                Some(bound) => PathBuf::from(bound),
                None => {
                    let dir = roots
                        .message_workspaces
                        .join(&agent.id)
                        .join(requester.directory_name());
                    create_private_dir(&dir)?;
                    dir
                }
            };
            workspace.to_string_lossy().into_owned()
        }
        DispatchDelegationTaskKind::Coding => {
            let workspace = Path::new(request.workspace_path.trim())
                .canonicalize()
                .context("resolve the provisioned coding workspace")?;
            let worktrees = roots
                .worktrees
                .canonicalize()
                .context("resolve this device's dispatch worktrees")?;
            if !workspace.starts_with(&worktrees) {
                bail!("coding tasks run only in a worktree this device provisioned");
            }
            workspace.to_string_lossy().into_owned()
        }
    };
    Ok(())
}

/// Who delegated a job: a device of this account or a cross-account
/// connection. Each has its own grants and its own message directory.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::protocol::{DispatchDelegation, DispatchDelegationSource};
    use openbitfun_core::service::delegation::OpenAgent;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: OpenAgentStore,
        roots: DelegationRoots,
        bound_workspace: PathBuf,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = OpenAgentStore::new(dir.path().join("open-agents.json"));
        let roots = DelegationRoots {
            message_workspaces: dir.path().join("message-workspaces"),
            worktrees: dir.path().join("worktrees"),
            assistants: dir.path().join("personal_assistant"),
        };
        std::fs::create_dir_all(roots.worktrees.join("repo").join("job-1")).unwrap();
        let bound_workspace = dir.path().join("notes");
        std::fs::create_dir_all(&bound_workspace).unwrap();
        let open = |id: &str, kinds: Vec<DelegationTaskKind>, workspace: Option<&Path>| OpenAgent {
            id: id.to_string(),
            agent_type: "Standard".to_string(),
            display_name: id.to_string(),
            description: None,
            task_kinds: kinds,
            message_workspace: workspace.map(|path| path.to_string_lossy().into_owned()),
            same_account_devices: vec!["laptop".to_string()],
            connections: Vec::new(),
        };
        store
            .open(open("helper", vec![DelegationTaskKind::Message], None))
            .await
            .unwrap();
        store
            .open(open(
                "notes",
                vec![DelegationTaskKind::Message],
                Some(&bound_workspace),
            ))
            .await
            .unwrap();
        store
            .open(open("coder", vec![DelegationTaskKind::Coding], None))
            .await
            .unwrap();
        Fixture {
            _dir: dir,
            store,
            roots,
            bound_workspace,
        }
    }

    fn delegated(open_agent_id: &str, kind: DispatchDelegationTaskKind) -> DispatchSubmitRequest {
        DispatchSubmitRequest {
            protocol_version:
                openbitfun_services_core::dispatch_contract::DISPATCH_PROTOCOL_VERSION,
            job_id: "job-1".to_string(),
            session_id: "session-1".to_string(),
            workspace_path: String::new(),
            agent_type: "Standard".to_string(),
            prompt: "What changed?".to_string(),
            approval_policy: DispatchApprovalPolicy::Remote,
            model: None,
            reasoning_preset: None,
            title: None,
            attachments: Vec::new(),
            setup_audit: Vec::new(),
            delegation: Some(DispatchDelegation {
                open_agent_id: open_agent_id.to_string(),
                task_kind: kind,
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
        let assistant_root = fixture.roots.assistants.join("workspace-ann");
        std::fs::create_dir_all(&assistant_root).unwrap();
        fixture
            .store
            .open_assistant(
                &fixture.roots.assistants,
                OpenAgent {
                    id: "ann".to_string(),
                    agent_type: "assistant:workspace-ann".to_string(),
                    display_name: "Ann".to_string(),
                    description: None,
                    task_kinds: vec![DelegationTaskKind::Message],
                    message_workspace: None,
                    same_account_devices: vec!["laptop".to_string()],
                    connections: Vec::new(),
                },
                true,
            )
            .await
            .unwrap();
        let ask = |job_id: &str| {
            let mut request = delegated("ann", DispatchDelegationTaskKind::Message);
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
        chosen.workspace_path = fixture.bound_workspace.to_string_lossy().into_owned();
        assert!(admit_error(&fixture, chosen)
            .await
            .contains("omit workspacePath"));
        let mut injected = ask("job-3");
        injected.assistant_workspace = Some("/".to_string());
        assert!(admit_error(&fixture, injected)
            .await
            .contains("chosen by this device"));
        assert!(admit_error(
            &fixture,
            DispatchSubmitRequest {
                agent_type: "assistant:workspace-ann".to_string(),
                ..delegated("ann", DispatchDelegationTaskKind::Coding)
            }
        )
        .await
        .contains("does not accept"));

        // An assistant removed after it was opened refuses new tasks.
        std::fs::remove_dir_all(&assistant_root).unwrap();
        assert!(admit_error(&fixture, ask("job-4"))
            .await
            .contains("no longer exists"));
    }

    #[tokio::test]
    async fn message_tasks_run_in_a_directory_this_device_chooses() {
        let fixture = fixture().await;
        let mut request = delegated("helper", DispatchDelegationTaskKind::Message);
        admit(&mut request, &fixture.store, &fixture.roots)
            .await
            .unwrap();
        let expected = fixture
            .roots
            .message_workspaces
            .join("helper")
            .join("laptop");
        assert_eq!(request.workspace_path, expected.to_string_lossy());
        assert!(expected.is_dir());

        let mut bound = delegated("notes", DispatchDelegationTaskKind::Message);
        admit(&mut bound, &fixture.store, &fixture.roots)
            .await
            .unwrap();
        assert_eq!(
            bound.workspace_path,
            fixture.bound_workspace.to_string_lossy()
        );

        // A requester cannot choose a path on this device.
        let mut chosen = delegated("helper", DispatchDelegationTaskKind::Message);
        chosen.workspace_path = fixture.bound_workspace.to_string_lossy().into_owned();
        assert!(admit_error(&fixture, chosen)
            .await
            .contains("omit workspacePath"));
    }

    #[tokio::test]
    async fn connections_use_their_own_grants_directory_and_no_model_choice() {
        let fixture = fixture().await;
        let over = |connection: &str| {
            let mut request = delegated("helper", DispatchDelegationTaskKind::Message);
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
            .grant_connection("helper", "conn-1", false)
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
            .join("connection-conn-1");
        assert_eq!(request.workspace_path, expected.to_string_lossy());
        assert!(expected.is_dir());
        assert_eq!(request.model, None);
        assert_eq!(request.reasoning_preset, None);

        assert!(admit_error(&fixture, over("conn-2"))
            .await
            .contains("not granted to this connection"));

        // Another account never shares the owner's bound directory, or the
        // sessions in it.
        fixture
            .store
            .grant_connection("notes", "conn-1", false)
            .await
            .unwrap();
        let mut notes = over("conn-1");
        notes.delegation.as_mut().unwrap().open_agent_id = "notes".to_string();
        admit(&mut notes, &fixture.store, &fixture.roots)
            .await
            .unwrap();
        let own = fixture
            .roots
            .message_workspaces
            .join("notes")
            .join("connection-conn-1");
        assert_eq!(notes.workspace_path, own.to_string_lossy());
        assert_ne!(
            notes.workspace_path,
            fixture.bound_workspace.to_string_lossy()
        );

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
    async fn coding_tasks_run_only_in_provisioned_worktrees() {
        let fixture = fixture().await;
        let mut request = delegated("coder", DispatchDelegationTaskKind::Coding);
        request.workspace_path = fixture
            .roots
            .worktrees
            .join("repo")
            .join("job-1")
            .to_string_lossy()
            .into_owned();
        admit(&mut request, &fixture.store, &fixture.roots)
            .await
            .unwrap();

        let mut elsewhere = delegated("coder", DispatchDelegationTaskKind::Coding);
        elsewhere.workspace_path = fixture.bound_workspace.to_string_lossy().into_owned();
        assert!(admit_error(&fixture, elsewhere)
            .await
            .contains("worktree this device provisioned"));
    }

    #[tokio::test]
    async fn refusals_name_the_reason_and_create_nothing() {
        let fixture = fixture().await;

        let mut stranger = delegated("helper", DispatchDelegationTaskKind::Message);
        stranger.source_device_id = Some("phone".to_string());
        assert!(admit_error(&fixture, stranger)
            .await
            .contains("not granted"));

        let missing = delegated("nobody", DispatchDelegationTaskKind::Message);
        assert!(admit_error(&fixture, missing)
            .await
            .contains("does not exist"));

        let wrong_kind = delegated("helper", DispatchDelegationTaskKind::Coding);
        assert!(admit_error(&fixture, wrong_kind)
            .await
            .contains("does not accept"));

        let mut wrong_agent = delegated("helper", DispatchDelegationTaskKind::Message);
        wrong_agent.agent_type = "Ultimate".to_string();
        assert!(admit_error(&fixture, wrong_agent)
            .await
            .contains("runs agent 'Standard'"));

        let mut unattended = delegated("helper", DispatchDelegationTaskKind::Message);
        unattended.approval_policy = DispatchApprovalPolicy::Auto;
        assert!(admit_error(&fixture, unattended)
            .await
            .contains("remote approval policy"));

        let mut anonymous = delegated("helper", DispatchDelegationTaskKind::Message);
        anonymous.source_device_id = None;
        assert!(admit_error(&fixture, anonymous)
            .await
            .contains("another device"));

        // None of the refusals created a message workspace.
        assert!(!fixture.roots.message_workspaces.exists());
    }

    #[tokio::test]
    async fn ordinary_jobs_cannot_claim_a_source_device() {
        let fixture = fixture().await;
        let mut ordinary = delegated("helper", DispatchDelegationTaskKind::Message);
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
