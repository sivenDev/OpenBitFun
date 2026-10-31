//! Agents this device opens to other devices for delegated dispatch jobs.
//!
//! The open list is device-local: it lives beside the dispatch data rather than
//! in the shared configuration document, so a configuration backup restored on
//! another device never opens agents there. Entries are stored raw and parsed
//! one by one; an entry this build cannot read is kept on disk, reported with a
//! reason, and treated as closed.

use std::path::{Path, PathBuf};

use openbitfun_services_core::json_store::{JsonFileStore, JsonFileStoreError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::infrastructure::PathManager;

const DELEGATION_DIR: &str = "delegation";
const OPEN_AGENTS_FILE: &str = "open-agents.json";
const OPEN_AGENTS_SCHEMA_VERSION: u32 = 1;
/// Grants every device of this account.
pub const ALL_SAME_ACCOUNT_DEVICES: &str = "*";
const MAX_OPEN_AGENT_ID_LEN: usize = 64;
/// Turn context key a target sets on a delegated job's turns, naming the
/// device that asked. A turn that carries it must not delegate onward.
pub const DISPATCH_DELEGATION_CONTEXT_KEY: &str = "dispatchDelegation";
/// Job ids a requester gives the jobs it delegates. Its manual dispatch list
/// never adopts them: the delegating subagent already observes them.
pub const DELEGATED_JOB_ID_PREFIX: &str = "delegation-";

/// Agent type of an opened assistant: `assistant:<directory>` names one
/// directory under this device's assistant workspace base. Older targets do
/// not know the prefix and refuse it as an unknown mode.
pub const ASSISTANT_AGENT_TYPE_PREFIX: &str = "assistant:";
/// The agent mode an opened assistant's tasks run in.
pub const ASSISTANT_RUN_AGENT_TYPE: &str = "Claw";
const DEFAULT_ASSISTANT_DIRECTORY: &str = "workspace";
const NAMED_ASSISTANT_DIRECTORY_PREFIX: &str = "workspace-";
/// Directory under the product home that holds delegated task directories.
/// Nothing under it is a source of memories.
const DELEGATION_WORKSPACES_DIR: &str = "workspaces";

/// The assistant directory an `assistant:<directory>` agent type names, when
/// it is one this device could hold.
pub fn assistant_directory(agent_type: &str) -> Option<&str> {
    let directory = agent_type
        .trim()
        .strip_prefix(ASSISTANT_AGENT_TYPE_PREFIX)?;
    let valid = directory == DEFAULT_ASSISTANT_DIRECTORY
        || directory
            .strip_prefix(NAMED_ASSISTANT_DIRECTORY_PREFIX)
            .is_some_and(|id| {
                !id.is_empty()
                    && id.len() <= MAX_OPEN_AGENT_ID_LEN
                    && id
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            });
    valid.then_some(directory)
}

/// The agent type that opens the assistant in `directory`.
pub fn assistant_agent_type(directory: &str) -> String {
    format!("{ASSISTANT_AGENT_TYPE_PREFIX}{directory}")
}

/// Root of the assistant an `assistant:<directory>` agent type names.
pub fn assistant_workspace_root(path_manager: &PathManager, agent_type: &str) -> Option<PathBuf> {
    assistant_directory(agent_type).map(|directory| {
        path_manager
            .assistant_workspace_base_dir(None)
            .join(directory)
    })
}

/// An assistant this device could open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAssistant {
    /// The agent type that opens it (`assistant:<directory>`).
    pub agent_type: String,
    /// The assistant id, or `None` for the default assistant.
    pub assistant_id: Option<String>,
    pub root_path: PathBuf,
}

/// The assistants on this device, read from their directories so headless
/// hosts can list them without the workspace catalog.
pub fn local_assistants(path_manager: &PathManager) -> Vec<LocalAssistant> {
    let Ok(entries) = std::fs::read_dir(path_manager.assistant_workspace_base_dir(None)) else {
        return Vec::new();
    };
    let mut assistants: Vec<LocalAssistant> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let directory = entry.file_name().to_string_lossy().into_owned();
            let agent_type = assistant_agent_type(&directory);
            assistant_directory(&agent_type)?;
            Some(LocalAssistant {
                assistant_id: directory
                    .strip_prefix(NAMED_ASSISTANT_DIRECTORY_PREFIX)
                    .map(str::to_string),
                root_path: entry.path(),
                agent_type,
            })
        })
        .collect();
    assistants.sort_by(|left, right| left.agent_type.cmp(&right.agent_type));
    assistants
}

/// Parent of the directories delegated tasks run in.
pub fn delegation_workspaces_root(path_manager: &PathManager) -> PathBuf {
    path_manager
        .product_home_dir()
        .join(DELEGATION_DIR)
        .join(DELEGATION_WORKSPACES_DIR)
}

/// Whether `path` is a directory a delegated task ran in. Such sessions
/// belong to another account's or device's request and never feed memories.
pub fn is_delegation_workspace_path(path: &Path) -> bool {
    crate::infrastructure::try_get_path_manager_arc()
        .is_ok_and(|path_manager| path.starts_with(delegation_workspaces_root(&path_manager)))
}

/// How far the owner trusts the requester of a delegated task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationTrust {
    /// The default for another account: the task cannot reach the owner's
    /// desktop, other sessions, schedules or publishing, and gets no memory.
    #[default]
    Normal,
    /// The owner's own devices, or a connection the owner chose to trust
    /// fully: the task runs like the owner's own.
    Full,
}

impl DelegationTrust {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Full => "full",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// Turn context keys that carry the facts of a delegated turn. The target's
/// worker writes them into the `dispatchDelegation` turn metadata; the kernel
/// copies them into the turn and every subagent it starts.
pub const DELEGATION_ASSISTANT_CONTEXT_KEY: &str = "dispatchDelegationAssistant";
pub const DELEGATION_TASK_DIR_CONTEXT_KEY: &str = "dispatchDelegationTaskDir";
pub const DELEGATION_TRUST_CONTEXT_KEY: &str = "dispatchDelegationTrust";
const METADATA_ASSISTANT_FIELD: &str = "assistantWorkspace";
const METADATA_TASK_DIR_FIELD: &str = "taskWorkspace";
const METADATA_TRUST_FIELD: &str = "trust";

/// What the kernel knows about a turn another device delegated here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedTurnFacts {
    /// Root of the opened assistant the task runs as; read-only to it.
    pub assistant_workspace: Option<PathBuf>,
    /// The task's own directory, the only place it writes files.
    pub task_dir: Option<PathBuf>,
    pub trust: DelegationTrust,
}

impl DelegatedTurnFacts {
    /// Copies the facts from a turn's `dispatchDelegation` metadata into its
    /// context variables. A missing or unknown trust stays unset, which reads
    /// as [`DelegationTrust::Normal`].
    pub fn copy_metadata_to_context(
        metadata: Option<&Value>,
        context: &mut std::collections::HashMap<String, String>,
    ) {
        let Some(delegation) = metadata
            .and_then(|metadata| metadata.get(DISPATCH_DELEGATION_CONTEXT_KEY))
            .filter(|delegation| delegation.is_object())
        else {
            return;
        };
        context.insert(
            DISPATCH_DELEGATION_CONTEXT_KEY.to_string(),
            "true".to_string(),
        );
        for (field, key) in [
            (METADATA_ASSISTANT_FIELD, DELEGATION_ASSISTANT_CONTEXT_KEY),
            (METADATA_TASK_DIR_FIELD, DELEGATION_TASK_DIR_CONTEXT_KEY),
            (METADATA_TRUST_FIELD, DELEGATION_TRUST_CONTEXT_KEY),
        ] {
            if let Some(value) = delegation
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                context.insert(key.to_string(), value.to_string());
            }
        }
    }

    /// The metadata fields a target's worker adds for a delegated turn.
    pub fn metadata_fields(
        assistant_workspace: Option<&str>,
        task_dir: &str,
        trust: DelegationTrust,
    ) -> serde_json::Map<String, Value> {
        let mut fields = serde_json::Map::new();
        if let Some(assistant_workspace) = assistant_workspace {
            fields.insert(
                METADATA_ASSISTANT_FIELD.to_string(),
                Value::String(assistant_workspace.to_string()),
            );
        }
        fields.insert(
            METADATA_TASK_DIR_FIELD.to_string(),
            Value::String(task_dir.to_string()),
        );
        fields.insert(
            METADATA_TRUST_FIELD.to_string(),
            Value::String(trust.as_str().to_string()),
        );
        fields
    }

    /// The facts of a delegated turn, or `None` for any other turn.
    pub fn from_context(context: &std::collections::HashMap<String, String>) -> Option<Self> {
        if context
            .get(DISPATCH_DELEGATION_CONTEXT_KEY)
            .map(String::as_str)
            != Some("true")
        {
            return None;
        }
        let path = |key: &str| {
            context
                .get(key)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        Some(Self {
            assistant_workspace: path(DELEGATION_ASSISTANT_CONTEXT_KEY),
            task_dir: path(DELEGATION_TASK_DIR_CONTEXT_KEY),
            trust: context
                .get(DELEGATION_TRUST_CONTEXT_KEY)
                .and_then(|value| DelegationTrust::parse(value))
                .unwrap_or_default(),
        })
    }

    /// Context keys a subagent inherits from its delegated parent turn.
    pub const INHERITED_CONTEXT_KEYS: [&'static str; 3] = [
        DELEGATION_ASSISTANT_CONTEXT_KEY,
        DELEGATION_TASK_DIR_CONTEXT_KEY,
        DELEGATION_TRUST_CONTEXT_KEY,
    ];

    /// Whether the task may see the owner's private context: `USER.md` and
    /// the memory summary.
    pub fn includes_private_context(&self) -> bool {
        self.trust == DelegationTrust::Full
    }

    /// Narrows `restrictions` for this turn: a task that runs as an
    /// assistant writes only in its own directory, so it can never change
    /// the assistant's persona, instructions or memory.
    #[cfg(feature = "agent-runtime")]
    pub fn restrict_tools(
        &self,
        mut restrictions: crate::agentic::tools::ToolRuntimeRestrictions,
    ) -> crate::agentic::tools::ToolRuntimeRestrictions {
        if let (Some(_), Some(task_dir)) = (&self.assistant_workspace, &self.task_dir) {
            let root = task_dir.to_string_lossy().into_owned();
            let policy = &mut restrictions.path_policy;
            for roots in [
                &mut policy.write_roots,
                &mut policy.edit_roots,
                &mut policy.delete_roots,
            ] {
                if roots.is_empty() {
                    roots.push(root.clone());
                } else {
                    roots.retain(|existing| Path::new(existing).starts_with(task_dir));
                    if roots.is_empty() {
                        roots.push(root.clone());
                    }
                }
            }
        }
        restrictions
    }
}

/// What a delegated job may do on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationTaskKind {
    /// A question or lookup: no code is transferred.
    Message,
    /// Work on a Git baseline delivered by the requester.
    Coding,
    /// A kind written by a newer build; never granted by this one.
    #[serde(other)]
    Unknown,
}

impl DelegationTaskKind {
    /// The wire name, or `None` for a kind this build does not grant.
    pub fn wire_name(self) -> Option<&'static str> {
        match self {
            Self::Message => Some("message"),
            Self::Coding => Some("coding"),
            Self::Unknown => None,
        }
    }
}

/// One agent this device opens to other devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAgent {
    /// Stable id the requester addresses.
    pub id: String,
    /// The local agent that runs delegated jobs.
    #[serde(deserialize_with = "openbitfun_core_types::agent_identity::deserialize_agent_id")]
    pub agent_type: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub task_kinds: Vec<DelegationTaskKind>,
    /// Directory message jobs run in. Without one, each requester device gets
    /// its own empty directory owned by this device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_workspace: Option<String>,
    /// Same-account devices allowed to delegate; [`ALL_SAME_ACCOUNT_DEVICES`]
    /// allows every device of this account.
    #[serde(default)]
    pub same_account_devices: Vec<String>,
    /// Cross-account connections allowed to delegate, granted one by one.
    /// Managed only through [`OpenAgentStore::grant_connection`] and
    /// [`OpenAgentStore::revoke_connection`]; reopening the same agent keeps
    /// them, opening a different agent under the id drops them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionGrant>,
}

/// One cross-account connection an open agent is granted to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionGrant {
    pub connection_id: String,
    /// Limits the owner set for this connection; unset means none.
    #[serde(default, flatten)]
    pub limits: ConnectionLimits,
}

/// Most this device does for one connection with one open agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionLimits {
    /// Tasks that may run at once; more are refused at submission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent_jobs: Option<u32>,
    /// Minutes one turn of a task may run before it is stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_job_minutes: Option<u32>,
}

/// Bounds the owner can set, so a typo cannot disable a limit.
pub const MAX_CONNECTION_CONCURRENT_JOBS: u32 = 100;
pub const MAX_CONNECTION_JOB_MINUTES: u32 = 24 * 60;

impl ConnectionLimits {
    fn validate(&self) -> Result<(), String> {
        if let Some(jobs) = self.max_concurrent_jobs {
            if !(1..=MAX_CONNECTION_CONCURRENT_JOBS).contains(&jobs) {
                return Err(format!(
                    "concurrent tasks must be 1-{MAX_CONNECTION_CONCURRENT_JOBS}"
                ));
            }
        }
        if let Some(minutes) = self.max_job_minutes {
            if !(1..=MAX_CONNECTION_JOB_MINUTES).contains(&minutes) {
                return Err(format!(
                    "task minutes must be 1-{MAX_CONNECTION_JOB_MINUTES}"
                ));
            }
        }
        Ok(())
    }
}

impl OpenAgent {
    pub fn allows_task_kind(&self, kind: DelegationTaskKind) -> bool {
        kind != DelegationTaskKind::Unknown && self.task_kinds.contains(&kind)
    }

    pub fn is_granted_to_device(&self, device_id: &str) -> bool {
        !device_id.trim().is_empty()
            && self
                .same_account_devices
                .iter()
                .any(|device| device == ALL_SAME_ACCOUNT_DEVICES || device == device_id)
    }

    pub fn is_granted_to_connection(&self, connection_id: &str) -> bool {
        self.connection_grant(connection_id).is_some()
    }

    /// The grant to `connection_id`, with its limits.
    pub fn connection_grant(&self, connection_id: &str) -> Option<&ConnectionGrant> {
        if connection_id.trim().is_empty() {
            return None;
        }
        self.connections
            .iter()
            .find(|grant| grant.connection_id == connection_id)
    }

    fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > MAX_OPEN_AGENT_ID_LEN
            || !self
                .id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return Err(format!(
                "open agent id must be 1-{MAX_OPEN_AGENT_ID_LEN} letters, digits, '-' or '_'"
            ));
        }
        let agent_type = self.agent_type.trim();
        if agent_type.is_empty() {
            return Err("open agent requires an agent type".to_string());
        }
        // Delegated jobs never reach another device.
        if agent_type.starts_with("remote:") {
            return Err(format!(
                "agent type '{agent_type}' cannot be opened to other devices"
            ));
        }
        if agent_type.starts_with(EXTERNAL_AGENT_TYPE_PREFIX)
            && external_client_id(agent_type).is_none()
        {
            return Err("external agent type requires a client id".to_string());
        }
        if agent_type.starts_with(ASSISTANT_AGENT_TYPE_PREFIX) {
            if assistant_directory(agent_type).is_none() {
                return Err(format!("'{agent_type}' does not name an assistant"));
            }
            // An assistant answers questions in a directory of each task's
            // own; it never takes a repository or a shared directory.
            if self
                .task_kinds
                .iter()
                .any(|kind| *kind != DelegationTaskKind::Message)
            {
                return Err("an opened assistant accepts only message tasks".to_string());
            }
            if self.message_workspace.is_some() {
                return Err("an opened assistant runs each task in its own directory".to_string());
            }
        }
        if self.display_name.trim().is_empty() {
            return Err("open agent requires a display name".to_string());
        }
        if !self
            .task_kinds
            .iter()
            .any(|kind| *kind != DelegationTaskKind::Unknown)
        {
            return Err("open agent must allow at least one task kind".to_string());
        }
        if let Some(workspace) = &self.message_workspace {
            if !Path::new(workspace).is_absolute() {
                return Err("message workspace must be an absolute path".to_string());
            }
        }
        if self
            .same_account_devices
            .iter()
            .any(|device| device.trim().is_empty())
        {
            return Err("granted device ids must not be empty".to_string());
        }
        for (index, grant) in self.connections.iter().enumerate() {
            if grant.connection_id.trim().is_empty() {
                return Err("granted connection ids must not be empty".to_string());
            }
            grant.limits.validate()?;
            if self.connections[..index]
                .iter()
                .any(|earlier| earlier.connection_id == grant.connection_id)
            {
                return Err(format!(
                    "connection '{}' is granted more than once",
                    grant.connection_id
                ));
            }
        }
        Ok(())
    }
}

const EXTERNAL_AGENT_TYPE_PREFIX: &str = "acp:";

/// The ACP client an `acp:<client_id>` agent type names.
fn external_client_id(agent_type: &str) -> Option<&str> {
    agent_type
        .strip_prefix(EXTERNAL_AGENT_TYPE_PREFIX)
        .map(str::trim)
        .filter(|client_id| !client_id.is_empty())
}

/// What the owner confirms before opening an external agent: delegated jobs
/// run it with this device's own login or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentUsageNotice {
    /// Jobs use the owner's quota with the agent's provider; its terms apply.
    ProviderQuota,
    /// As `ProviderQuota`, and a Claude subscription login is for the owner's
    /// own use: an API key is the recommended login for delegated jobs.
    ClaudeCodeSubscription,
}

impl ExternalAgentUsageNotice {
    pub fn for_agent_type(agent_type: &str) -> Option<Self> {
        external_client_id(agent_type.trim()).map(|client_id| {
            if client_id == "claude-code" {
                Self::ClaudeCodeSubscription
            } else {
                Self::ProviderQuota
            }
        })
    }
}

/// One stored entry, readable or not.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAgentRecord {
    /// Present when this build can read and accept the entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<OpenAgent>,
    /// Why the entry is treated as closed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// The stored entry when it is unreadable, so management surfaces can
    /// still show and close it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

impl OpenAgentRecord {
    fn parse(raw: Value) -> Self {
        let parsed = serde_json::from_value::<OpenAgent>(raw.clone())
            .map_err(|error| format!("unreadable entry: {error}"))
            .and_then(|agent| agent.validate().map(|()| agent));
        match parsed {
            Ok(agent) => Self {
                agent: Some(agent),
                problem: None,
                raw: None,
            },
            Err(problem) => Self {
                agent: None,
                problem: Some(problem),
                raw: Some(raw),
            },
        }
    }

    /// The entry id, even for an unreadable entry that stores one.
    pub fn id(&self) -> Option<&str> {
        self.agent
            .as_ref()
            .map(|agent| agent.id.as_str())
            .or_else(|| {
                self.raw
                    .as_ref()
                    .and_then(|raw| raw.get("id"))
                    .and_then(Value::as_str)
            })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenAgentsDocument {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    entries: Vec<Value>,
}

/// Why a delegated job is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DelegationRefusal {
    #[error("open agent '{0}' does not exist on this device")]
    UnknownOpenAgent(String),
    #[error("open agent '{0}' is not granted to the requesting device")]
    NotGranted(String),
    #[error("open agent '{open_agent_id}' does not accept {kind:?} tasks")]
    TaskKindNotAllowed {
        open_agent_id: String,
        kind: DelegationTaskKind,
    },
    #[error("the requesting device is unknown")]
    MissingRequester,
    #[error("open agent '{0}' is not granted to this connection")]
    NotGrantedToConnection(String),
}

#[derive(Debug, Error)]
pub enum OpenAgentStoreError {
    #[error(transparent)]
    Store(#[from] JsonFileStoreError),
    #[error("{0}")]
    Invalid(String),
}

/// Device-local store of opened agents.
#[derive(Debug, Clone)]
pub struct OpenAgentStore {
    path: PathBuf,
}

impl OpenAgentStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_product_home(path_manager: &PathManager) -> Self {
        Self::new(
            path_manager
                .product_home_dir()
                .join(DELEGATION_DIR)
                .join(OPEN_AGENTS_FILE),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every stored entry, including ones treated as closed.
    pub async fn records(&self) -> Result<Vec<OpenAgentRecord>, OpenAgentStoreError> {
        let document: OpenAgentsDocument = JsonFileStore
            .read_locked_optional(&self.path)
            .await?
            .unwrap_or_default();
        Ok(document
            .entries
            .into_iter()
            .map(OpenAgentRecord::parse)
            .collect())
    }

    /// Readable entries granted to `device_id`.
    pub async fn granted_to_device(
        &self,
        device_id: &str,
    ) -> Result<Vec<OpenAgent>, OpenAgentStoreError> {
        Ok(self
            .records()
            .await?
            .into_iter()
            .filter_map(|record| record.agent)
            .filter(|agent| agent.is_granted_to_device(device_id))
            .collect())
    }

    /// The open agent a delegated job from `device_id` may run, or why not.
    pub async fn authorize(
        &self,
        device_id: &str,
        open_agent_id: &str,
        kind: DelegationTaskKind,
    ) -> Result<Result<OpenAgent, DelegationRefusal>, OpenAgentStoreError> {
        if device_id.trim().is_empty() {
            return Ok(Err(DelegationRefusal::MissingRequester));
        }
        let Some(agent) = self
            .records()
            .await?
            .into_iter()
            .filter_map(|record| record.agent)
            .find(|agent| agent.id == open_agent_id)
        else {
            return Ok(Err(DelegationRefusal::UnknownOpenAgent(
                open_agent_id.to_string(),
            )));
        };
        if !agent.is_granted_to_device(device_id) {
            return Ok(Err(DelegationRefusal::NotGranted(agent.id)));
        }
        if !agent.allows_task_kind(kind) {
            return Ok(Err(DelegationRefusal::TaskKindNotAllowed {
                open_agent_id: agent.id,
                kind,
            }));
        }
        Ok(Ok(agent))
    }

    /// Readable entries granted to the cross-account connection.
    pub async fn granted_to_connection(
        &self,
        connection_id: &str,
    ) -> Result<Vec<OpenAgent>, OpenAgentStoreError> {
        Ok(self
            .records()
            .await?
            .into_iter()
            .filter_map(|record| record.agent)
            .filter(|agent| agent.is_granted_to_connection(connection_id))
            .collect())
    }

    /// The open agent a delegated job over `connection_id` may run, or why
    /// not. Same-account device grants never apply to a connection.
    pub async fn authorize_connection(
        &self,
        connection_id: &str,
        open_agent_id: &str,
        kind: DelegationTaskKind,
    ) -> Result<Result<OpenAgent, DelegationRefusal>, OpenAgentStoreError> {
        if connection_id.trim().is_empty() {
            return Ok(Err(DelegationRefusal::MissingRequester));
        }
        let Some(agent) = self
            .records()
            .await?
            .into_iter()
            .filter_map(|record| record.agent)
            .find(|agent| agent.id == open_agent_id)
        else {
            return Ok(Err(DelegationRefusal::UnknownOpenAgent(
                open_agent_id.to_string(),
            )));
        };
        if !agent.is_granted_to_connection(connection_id) {
            return Ok(Err(DelegationRefusal::NotGrantedToConnection(agent.id)));
        }
        if !agent.allows_task_kind(kind) {
            return Ok(Err(DelegationRefusal::TaskKindNotAllowed {
                open_agent_id: agent.id,
                kind,
            }));
        }
        Ok(Ok(agent))
    }

    /// Grants the open agent `open_agent_id` to a cross-account connection.
    /// An external agent is granted only after the owner confirmed its usage
    /// notice again for this connection.
    pub async fn grant_connection(
        &self,
        open_agent_id: &str,
        connection_id: &str,
        usage_confirmed: bool,
    ) -> Result<OpenAgent, OpenAgentStoreError> {
        let connection_id = connection_id.trim();
        if connection_id.is_empty() {
            return Err(OpenAgentStoreError::Invalid(
                "a connection id is required".to_string(),
            ));
        }
        let mut agent = self.readable(open_agent_id).await?;
        if ExternalAgentUsageNotice::for_agent_type(&agent.agent_type).is_some() && !usage_confirmed
        {
            return Err(OpenAgentStoreError::Invalid(
                "granting an external agent to another account requires confirming its usage notice"
                    .to_string(),
            ));
        }
        if !agent.is_granted_to_connection(connection_id) {
            agent.connections.push(ConnectionGrant {
                connection_id: connection_id.to_string(),
                limits: ConnectionLimits::default(),
            });
            self.write_entry(agent.clone()).await?;
        }
        Ok(agent)
    }

    /// Sets the limits of an existing grant; unset fields remove a limit.
    pub async fn set_connection_limits(
        &self,
        open_agent_id: &str,
        connection_id: &str,
        limits: ConnectionLimits,
    ) -> Result<OpenAgent, OpenAgentStoreError> {
        limits.validate().map_err(OpenAgentStoreError::Invalid)?;
        let mut agent = self.readable(open_agent_id).await?;
        let grant = agent
            .connections
            .iter_mut()
            .find(|grant| grant.connection_id == connection_id)
            .ok_or_else(|| {
                OpenAgentStoreError::Invalid(format!(
                    "open agent '{open_agent_id}' is not granted to connection '{connection_id}'"
                ))
            })?;
        if grant.limits != limits {
            grant.limits = limits;
            self.write_entry(agent.clone()).await?;
        }
        Ok(agent)
    }

    /// Withdraws the connection's grant. Returns whether one existed.
    pub async fn revoke_connection(
        &self,
        open_agent_id: &str,
        connection_id: &str,
    ) -> Result<bool, OpenAgentStoreError> {
        let mut agent = self.readable(open_agent_id).await?;
        let before = agent.connections.len();
        agent
            .connections
            .retain(|grant| grant.connection_id != connection_id);
        if agent.connections.len() == before {
            return Ok(false);
        }
        self.write_entry(agent).await?;
        Ok(true)
    }

    async fn readable(&self, open_agent_id: &str) -> Result<OpenAgent, OpenAgentStoreError> {
        self.records()
            .await?
            .into_iter()
            .filter_map(|record| record.agent)
            .find(|agent| agent.id == open_agent_id)
            .ok_or_else(|| {
                OpenAgentStoreError::Invalid(format!(
                    "open agent '{open_agent_id}' does not exist on this device"
                ))
            })
    }

    /// Opens `agent`, replacing any stored entry with the same id. External
    /// agents are opened only through [`Self::open_external_agent`].
    pub async fn open(&self, agent: OpenAgent) -> Result<(), OpenAgentStoreError> {
        if assistant_directory(&agent.agent_type).is_some() {
            return Err(OpenAgentStoreError::Invalid(
                "assistants are opened only through open_assistant".to_string(),
            ));
        }
        if ExternalAgentUsageNotice::for_agent_type(&agent.agent_type).is_some() {
            return Err(OpenAgentStoreError::Invalid(
                "external agents are opened only after confirming their usage notice".to_string(),
            ));
        }
        self.store_entry(agent).await
    }

    /// Opens an `acp:` external agent after the owner confirmed its usage
    /// notice and `executor` reports that this device can run it now.
    #[cfg(feature = "agent-runtime")]
    pub async fn open_external_agent(
        &self,
        mut agent: OpenAgent,
        usage_confirmed: bool,
        executor: Option<&dyn openbitfun_runtime_ports::ExternalAgentExecutor>,
    ) -> Result<OpenAgent, OpenAgentStoreError> {
        agent.agent_type = agent.agent_type.trim().to_string();
        let Some(client_id) = external_client_id(&agent.agent_type).map(str::to_string) else {
            return Err(OpenAgentStoreError::Invalid(format!(
                "agent '{}' is not an external agent",
                agent.agent_type
            )));
        };
        if !usage_confirmed {
            return Err(OpenAgentStoreError::Invalid(
                "opening an external agent requires confirming its usage notice".to_string(),
            ));
        }
        let Some(executor) = executor else {
            return Err(OpenAgentStoreError::Invalid(
                "this device cannot run external agents".to_string(),
            ));
        };
        let context = openbitfun_runtime_ports::ExternalAgentContext {
            openbitfun_session_id: String::new(),
            workspace_path: None,
            remote_connection_id: None,
            session_storage_path: None,
        };
        if let openbitfun_runtime_ports::ExternalAgentAvailability::Unavailable { detail, .. } =
            executor.availability(&client_id, &context).await
        {
            return Err(OpenAgentStoreError::Invalid(format!(
                "external agent '{client_id}' cannot run on this device: {detail}"
            )));
        }
        if agent.display_name.trim().is_empty() {
            agent.display_name = client_id;
        }
        self.store_entry(agent.clone()).await?;
        Ok(agent)
    }

    /// Stores an opened agent. Connection grants are not part of opening:
    /// the stored entry's grants carry over while it runs the same agent.
    async fn store_entry(&self, mut agent: OpenAgent) -> Result<(), OpenAgentStoreError> {
        agent.agent_type =
            openbitfun_core_types::agent_identity::canonical_agent_id(agent.agent_type.trim())
                .to_string();
        agent.connections = self
            .records()
            .await?
            .into_iter()
            .filter_map(|record| record.agent)
            .find(|stored| stored.id == agent.id && stored.agent_type == agent.agent_type)
            .map(|stored| stored.connections)
            .unwrap_or_default();
        self.write_entry(agent).await
    }

    async fn write_entry(&self, mut agent: OpenAgent) -> Result<(), OpenAgentStoreError> {
        let mut task_kinds = Vec::new();
        for kind in agent.task_kinds {
            if kind != DelegationTaskKind::Unknown && !task_kinds.contains(&kind) {
                task_kinds.push(kind);
            }
        }
        agent.task_kinds = task_kinds;
        agent.validate().map_err(OpenAgentStoreError::Invalid)?;
        let raw = serde_json::to_value(&agent)
            .map_err(|error| OpenAgentStoreError::Invalid(error.to_string()))?;
        JsonFileStore
            .update_locked(
                &self.path,
                OpenAgentsDocument::default(),
                |document: &mut OpenAgentsDocument| {
                    document.schema_version = OPEN_AGENTS_SCHEMA_VERSION;
                    match document
                        .entries
                        .iter_mut()
                        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(&agent.id))
                    {
                        Some(entry) => *entry = raw,
                        None => document.entries.push(raw),
                    }
                },
            )
            .await?;
        Ok(())
    }

    /// Opens the assistant `agent.agent_type` names, under `assistants_base`
    /// (see [`PathManager::assistant_workspace_base_dir`]). The owner confirms
    /// once that tasks may use external agents with this device's own login
    /// or credentials. An empty display name takes the assistant's directory.
    pub async fn open_assistant(
        &self,
        assistants_base: &Path,
        mut agent: OpenAgent,
        usage_confirmed: bool,
    ) -> Result<OpenAgent, OpenAgentStoreError> {
        agent.agent_type = agent.agent_type.trim().to_string();
        let root = assistant_directory(&agent.agent_type)
            .map(|directory| assistants_base.join(directory))
            .ok_or_else(|| {
                OpenAgentStoreError::Invalid(format!(
                    "'{}' does not name an assistant",
                    agent.agent_type
                ))
            })?;
        if !root.is_dir() {
            return Err(OpenAgentStoreError::Invalid(format!(
                "assistant '{}' does not exist on this device",
                agent.agent_type
            )));
        }
        if !usage_confirmed {
            return Err(OpenAgentStoreError::Invalid(
                "opening an assistant requires confirming that its tasks may use external agents with this device's own login or credentials".to_string(),
            ));
        }
        agent.task_kinds = vec![DelegationTaskKind::Message];
        agent.message_workspace = None;
        if agent.display_name.trim().is_empty() {
            agent.display_name = assistant_directory(&agent.agent_type)
                .unwrap_or_default()
                .to_string();
        }
        self.store_entry(agent.clone()).await?;
        Ok(agent)
    }

    /// Opens a local agent mode after resolving its canonical id against the
    /// agent registry. An empty display name takes the mode's name.
    #[cfg(feature = "agent-runtime")]
    pub async fn open_local_agent(
        &self,
        mut agent: OpenAgent,
    ) -> Result<OpenAgent, OpenAgentStoreError> {
        agent.agent_type =
            openbitfun_core_types::agent_identity::canonical_agent_id(agent.agent_type.trim())
                .to_string();
        let mode = crate::agentic::get_agent_registry()
            .get_modes_info()
            .await
            .into_iter()
            .find(|mode| mode.id == agent.agent_type)
            .ok_or_else(|| {
                OpenAgentStoreError::Invalid(format!(
                    "agent '{}' is not a local agent mode",
                    agent.agent_type
                ))
            })?;
        if agent.display_name.trim().is_empty() {
            agent.display_name = mode.name;
        }
        self.open(agent.clone()).await?;
        Ok(agent)
    }

    /// Closes the entry with `id`, readable or not. Returns whether one existed.
    pub async fn close(&self, id: &str) -> Result<bool, OpenAgentStoreError> {
        let (removed, _) = JsonFileStore
            .update_locked(
                &self.path,
                OpenAgentsDocument::default(),
                |document: &mut OpenAgentsDocument| {
                    let before = document.entries.len();
                    document
                        .entries
                        .retain(|entry| OpenAgentRecord::parse(entry.clone()).id() != Some(id));
                    before != document.entries.len()
                },
            )
            .await?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(id: &str, devices: &[&str]) -> OpenAgent {
        OpenAgent {
            id: id.to_string(),
            agent_type: "agentic".to_string(),
            display_name: "Helper".to_string(),
            description: None,
            task_kinds: vec![DelegationTaskKind::Message],
            message_workspace: None,
            same_account_devices: devices.iter().map(|device| device.to_string()).collect(),
            connections: Vec::new(),
        }
    }

    fn store() -> (OpenAgentStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (
            OpenAgentStore::new(dir.path().join("delegation").join(OPEN_AGENTS_FILE)),
            dir,
        )
    }

    #[tokio::test]
    async fn nothing_is_open_by_default() {
        let (store, _dir) = store();
        assert!(store.records().await.unwrap().is_empty());
        assert!(store.granted_to_device("laptop").await.unwrap().is_empty());
        assert_eq!(
            store
                .authorize("laptop", "helper", DelegationTaskKind::Message)
                .await
                .unwrap(),
            Err(DelegationRefusal::UnknownOpenAgent("helper".to_string()))
        );
    }

    #[tokio::test]
    async fn grants_are_per_device_and_per_task_kind() {
        let (store, _dir) = store();
        store.open(agent("helper", &["laptop"])).await.unwrap();
        store
            .open(agent("everyone", &[ALL_SAME_ACCOUNT_DEVICES]))
            .await
            .unwrap();

        let laptop: Vec<_> = store
            .granted_to_device("laptop")
            .await
            .unwrap()
            .into_iter()
            .map(|agent| agent.id)
            .collect();
        assert_eq!(laptop, vec!["helper", "everyone"]);
        let phone: Vec<_> = store
            .granted_to_device("phone")
            .await
            .unwrap()
            .into_iter()
            .map(|agent| agent.id)
            .collect();
        assert_eq!(phone, vec!["everyone"]);

        assert!(store
            .authorize("laptop", "helper", DelegationTaskKind::Message)
            .await
            .unwrap()
            .is_ok());
        assert_eq!(
            store
                .authorize("phone", "helper", DelegationTaskKind::Message)
                .await
                .unwrap(),
            Err(DelegationRefusal::NotGranted("helper".to_string()))
        );
        assert_eq!(
            store
                .authorize("laptop", "helper", DelegationTaskKind::Coding)
                .await
                .unwrap(),
            Err(DelegationRefusal::TaskKindNotAllowed {
                open_agent_id: "helper".to_string(),
                kind: DelegationTaskKind::Coding,
            })
        );
        assert_eq!(
            store
                .authorize("", "everyone", DelegationTaskKind::Message)
                .await
                .unwrap(),
            Err(DelegationRefusal::MissingRequester)
        );
    }

    #[tokio::test]
    async fn connections_are_granted_one_by_one_and_apart_from_devices() {
        let (store, _dir) = store();
        store.open(agent("helper", &["*"])).await.unwrap();
        store.open(agent("reviewer", &[])).await.unwrap();

        // Nothing is granted to a connection by default, not even "*".
        assert!(store
            .granted_to_connection("conn-1")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .authorize_connection("conn-1", "helper", DelegationTaskKind::Message)
                .await
                .unwrap(),
            Err(DelegationRefusal::NotGrantedToConnection(
                "helper".to_string()
            ))
        );

        store
            .grant_connection("reviewer", "conn-1", false)
            .await
            .unwrap();
        let granted = store.granted_to_connection("conn-1").await.unwrap();
        assert_eq!(granted.len(), 1);
        assert_eq!(granted[0].id, "reviewer");
        assert!(store
            .granted_to_connection("conn-2")
            .await
            .unwrap()
            .is_empty());
        assert!(store
            .authorize_connection("conn-1", "reviewer", DelegationTaskKind::Message)
            .await
            .unwrap()
            .is_ok());
        assert_eq!(
            store
                .authorize_connection("conn-1", "reviewer", DelegationTaskKind::Coding)
                .await
                .unwrap(),
            Err(DelegationRefusal::TaskKindNotAllowed {
                open_agent_id: "reviewer".to_string(),
                kind: DelegationTaskKind::Coding,
            })
        );
        // A connection grant never admits a same-account device, and the
        // device id "conn-1" is not the connection.
        assert_eq!(
            store
                .authorize("conn-1", "reviewer", DelegationTaskKind::Message)
                .await
                .unwrap(),
            Err(DelegationRefusal::NotGranted("reviewer".to_string()))
        );

        // Granting twice keeps one grant; revoking removes it.
        store
            .grant_connection("reviewer", "conn-1", false)
            .await
            .unwrap();
        assert_eq!(
            store.granted_to_connection("conn-1").await.unwrap()[0]
                .connections
                .len(),
            1
        );
        assert!(store.revoke_connection("reviewer", "conn-1").await.unwrap());
        assert!(!store.revoke_connection("reviewer", "conn-1").await.unwrap());
        assert!(store
            .granted_to_connection("conn-1")
            .await
            .unwrap()
            .is_empty());
        assert!(store
            .grant_connection("missing", "conn-1", false)
            .await
            .is_err());
        assert!(store
            .grant_connection("reviewer", " ", false)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn connection_limits_are_set_per_grant_and_kept_on_regrant() {
        let (store, _dir) = store();
        store.open(agent("helper", &[])).await.unwrap();
        let limits = ConnectionLimits {
            max_concurrent_jobs: Some(2),
            max_job_minutes: Some(30),
        };
        // Limits apply only to an existing grant.
        assert!(store
            .set_connection_limits("helper", "conn-1", limits)
            .await
            .is_err());
        store
            .grant_connection("helper", "conn-1", false)
            .await
            .unwrap();
        store
            .set_connection_limits("helper", "conn-1", limits)
            .await
            .unwrap();
        // Granting again keeps them.
        let agent = store
            .grant_connection("helper", "conn-1", false)
            .await
            .unwrap();
        assert_eq!(agent.connection_grant("conn-1").unwrap().limits, limits);
        for invalid in [
            ConnectionLimits {
                max_concurrent_jobs: Some(0),
                max_job_minutes: None,
            },
            ConnectionLimits {
                max_concurrent_jobs: None,
                max_job_minutes: Some(MAX_CONNECTION_JOB_MINUTES + 1),
            },
        ] {
            assert!(store
                .set_connection_limits("helper", "conn-1", invalid)
                .await
                .is_err());
        }
        // Stored flat beside the connection id; a grant without them reads.
        let raw = std::fs::read_to_string(store.path()).unwrap();
        assert!(raw.contains("\"maxConcurrentJobs\":2"), "{raw}");
        let legacy: ConnectionGrant =
            serde_json::from_value(serde_json::json!({ "connectionId": "conn-2" })).unwrap();
        assert_eq!(legacy.limits, ConnectionLimits::default());
    }

    #[tokio::test]
    async fn reopening_keeps_connection_grants_unless_the_agent_changes() {
        let (store, _dir) = store();
        store.open(agent("helper", &[])).await.unwrap();
        store
            .grant_connection("helper", "conn-1", false)
            .await
            .unwrap();

        let mut edited = agent("helper", &["laptop"]);
        edited.display_name = "Renamed".to_string();
        store.open(edited).await.unwrap();
        assert_eq!(
            store.granted_to_connection("conn-1").await.unwrap().len(),
            1
        );

        let mut other = agent("helper", &[]);
        other.agent_type = "Plan".to_string();
        store.open(other).await.unwrap();
        assert!(store
            .granted_to_connection("conn-1")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn reopening_replaces_and_closing_removes() {
        let (store, _dir) = store();
        store.open(agent("helper", &["laptop"])).await.unwrap();
        store
            .open(OpenAgent {
                task_kinds: vec![
                    DelegationTaskKind::Message,
                    DelegationTaskKind::Unknown,
                    DelegationTaskKind::Message,
                ],
                ..agent("helper", &["phone"])
            })
            .await
            .unwrap();
        let records = store.records().await.unwrap();
        assert_eq!(records.len(), 1);
        let reopened = records[0].agent.as_ref().unwrap();
        assert_eq!(reopened.same_account_devices, vec!["phone"]);
        assert_eq!(reopened.agent_type, "Standard");
        assert_eq!(reopened.task_kinds, vec![DelegationTaskKind::Message]);
        assert!(store.close("helper").await.unwrap());
        assert!(!store.close("helper").await.unwrap());
        assert!(store.records().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn invalid_agents_are_refused() {
        let (store, _dir) = store();
        for invalid in [
            OpenAgent {
                id: "bad id".to_string(),
                ..agent("x", &["laptop"])
            },
            OpenAgent {
                agent_type: "acp:".to_string(),
                ..agent("x", &["laptop"])
            },
            OpenAgent {
                agent_type: "remote:device/helper".to_string(),
                ..agent("x", &["laptop"])
            },
            OpenAgent {
                task_kinds: vec![DelegationTaskKind::Unknown],
                ..agent("x", &["laptop"])
            },
            OpenAgent {
                message_workspace: Some("relative/dir".to_string()),
                ..agent("x", &["laptop"])
            },
        ] {
            assert!(matches!(
                store.open(invalid).await,
                Err(OpenAgentStoreError::Invalid(_))
            ));
        }
        assert!(store.records().await.unwrap().is_empty());
    }

    /// Reports `client_id` available when listed, otherwise not enabled.
    #[cfg(feature = "agent-runtime")]
    struct Clients(&'static [&'static str]);

    #[cfg(feature = "agent-runtime")]
    #[async_trait::async_trait]
    impl openbitfun_runtime_ports::ExternalAgentExecutor for Clients {
        async fn availability(
            &self,
            client_id: &str,
            _context: &openbitfun_runtime_ports::ExternalAgentContext,
        ) -> openbitfun_runtime_ports::ExternalAgentAvailability {
            if self.0.contains(&client_id) {
                openbitfun_runtime_ports::ExternalAgentAvailability::Available
            } else {
                openbitfun_runtime_ports::ExternalAgentAvailability::Unavailable {
                    reason: openbitfun_runtime_ports::ExternalAgentUnavailableReason::NotEnabled,
                    detail: format!("{client_id} is not enabled"),
                }
            }
        }

        async fn run_turn(
            &self,
            _request: openbitfun_runtime_ports::ExternalAgentTurnRequest,
            _sink: &dyn openbitfun_runtime_ports::ExternalAgentEventSink,
        ) -> openbitfun_runtime_ports::PortResult<openbitfun_runtime_ports::ExternalAgentTurnOutcome>
        {
            unreachable!("opening never runs the agent")
        }

        async fn cancel(
            &self,
            _client_id: &str,
            _session_id: &str,
        ) -> openbitfun_runtime_ports::PortResult<bool> {
            Ok(false)
        }

        async fn close(
            &self,
            _client_id: &str,
            _session_id: &str,
        ) -> openbitfun_runtime_ports::PortResult<()> {
            Ok(())
        }
    }

    #[cfg(feature = "agent-runtime")]
    fn external(id: &str, agent_type: &str) -> OpenAgent {
        OpenAgent {
            agent_type: agent_type.to_string(),
            display_name: String::new(),
            ..agent(id, &["laptop"])
        }
    }

    #[cfg(feature = "agent-runtime")]
    #[tokio::test]
    async fn external_agents_open_only_when_confirmed_and_runnable() {
        let (store, _dir) = store();
        let codex = Clients(&["codex"]);
        let refused = |result: Result<OpenAgent, OpenAgentStoreError>| match result {
            Err(OpenAgentStoreError::Invalid(reason)) => reason,
            other => panic!("expected a refusal, got {other:?}"),
        };

        // The plain open path never stores an external agent.
        assert!(matches!(
            store.open(external("codex", "acp:codex")).await,
            Err(OpenAgentStoreError::Invalid(_))
        ));
        assert!(refused(
            store
                .open_external_agent(external("codex", "acp:codex"), false, Some(&codex))
                .await
        )
        .contains("confirming"));
        assert!(refused(
            store
                .open_external_agent(external("codex", "acp:codex"), true, None)
                .await
        )
        .contains("cannot run external agents"));
        assert!(refused(
            store
                .open_external_agent(external("claude", "acp:claude-code"), true, Some(&codex))
                .await
        )
        .contains("claude-code is not enabled"));
        assert!(refused(
            store
                .open_external_agent(external("helper", "Standard"), true, Some(&codex))
                .await
        )
        .contains("not an external agent"));
        assert!(store.records().await.unwrap().is_empty());

        let opened = store
            .open_external_agent(external("codex", " acp:codex "), true, Some(&codex))
            .await
            .unwrap();
        assert_eq!(opened.agent_type, "acp:codex");
        assert_eq!(opened.display_name, "codex");
        let granted = store.granted_to_device("laptop").await.unwrap();
        assert_eq!(granted.len(), 1);
        assert_eq!(granted[0].agent_type, "acp:codex");

        // Another account gets an external agent only after a fresh
        // confirmation for that connection.
        assert!(
            refused(store.grant_connection("codex", "conn-1", false).await)
                .contains("requires confirming its usage notice")
        );
        assert!(store
            .granted_to_connection("conn-1")
            .await
            .unwrap()
            .is_empty());
        store
            .grant_connection("codex", "conn-1", true)
            .await
            .unwrap();
        assert_eq!(
            store.granted_to_connection("conn-1").await.unwrap().len(),
            1
        );
    }

    #[test]
    fn external_agents_name_their_usage_notice() {
        assert_eq!(
            ExternalAgentUsageNotice::for_agent_type("acp:claude-code"),
            Some(ExternalAgentUsageNotice::ClaudeCodeSubscription)
        );
        assert_eq!(
            ExternalAgentUsageNotice::for_agent_type("acp:codex"),
            Some(ExternalAgentUsageNotice::ProviderQuota)
        );
        assert_eq!(ExternalAgentUsageNotice::for_agent_type("Standard"), None);
        assert_eq!(ExternalAgentUsageNotice::for_agent_type("acp:"), None);
    }

    #[test]
    fn assistant_agent_types_name_only_assistant_directories() {
        assert_eq!(
            assistant_directory("assistant:workspace"),
            Some("workspace")
        );
        assert_eq!(
            assistant_directory("assistant:workspace-ab12_c"),
            Some("workspace-ab12_c")
        );
        for invalid in [
            "assistant:",
            "assistant:workspace-",
            "assistant:../workspace",
            "assistant:workspace-a/b",
            "assistant:notes",
            "Claw",
        ] {
            assert_eq!(assistant_directory(invalid), None, "{invalid}");
        }
        assert_eq!(assistant_agent_type("workspace-x"), "assistant:workspace-x");
    }

    fn assistant_entry(id: &str, agent_type: &str) -> OpenAgent {
        OpenAgent {
            agent_type: agent_type.to_string(),
            display_name: String::new(),
            ..agent(id, &[ALL_SAME_ACCOUNT_DEVICES])
        }
    }

    #[tokio::test]
    async fn assistants_open_only_with_confirmation_and_only_for_message_tasks() {
        let (store, dir) = store();
        let path_manager = PathManager::with_user_root_for_tests(dir.path().join("user"));
        let base = path_manager.assistant_workspace_base_dir(None);
        std::fs::create_dir_all(base.join("workspace-ann")).unwrap();
        std::fs::create_dir_all(base.join("notes")).unwrap();
        assert_eq!(
            local_assistants(&path_manager)
                .into_iter()
                .map(|assistant| assistant.agent_type)
                .collect::<Vec<_>>(),
            vec!["assistant:workspace-ann".to_string()]
        );

        let entry = assistant_entry("ann", "assistant:workspace-ann");
        assert!(store
            .open_assistant(&base, entry.clone(), false)
            .await
            .is_err());
        assert!(store
            .open_assistant(
                &base,
                assistant_entry("gone", "assistant:workspace-gone"),
                true
            )
            .await
            .is_err());
        // Opening ignores a requested coding kind or shared directory.
        let opened = store
            .open_assistant(
                &base,
                OpenAgent {
                    task_kinds: vec![DelegationTaskKind::Coding],
                    message_workspace: Some("/tmp".to_string()),
                    ..entry.clone()
                },
                true,
            )
            .await
            .unwrap();
        assert_eq!(opened.task_kinds, vec![DelegationTaskKind::Message]);
        assert_eq!(opened.message_workspace, None);
        assert_eq!(opened.display_name, "workspace-ann");
        assert!(store.open(entry).await.is_err());

        let granted = store
            .authorize("laptop", "ann", DelegationTaskKind::Message)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(granted.agent_type, "assistant:workspace-ann");
        assert_eq!(
            store
                .authorize("laptop", "ann", DelegationTaskKind::Coding)
                .await
                .unwrap(),
            Err(DelegationRefusal::TaskKindNotAllowed {
                open_agent_id: "ann".to_string(),
                kind: DelegationTaskKind::Coding,
            })
        );
    }

    #[test]
    fn delegated_turn_facts_travel_from_metadata_to_context() {
        let fields = DelegatedTurnFacts::metadata_fields(
            Some("/home/a/.openbitfun/personal_assistant/workspace"),
            "/home/a/.openbitfun/delegation/workspaces/ann/conn/job",
            DelegationTrust::Normal,
        );
        let mut delegation = json!({ "openAgentId": "ann" });
        delegation.as_object_mut().unwrap().extend(fields);
        let metadata = json!({ DISPATCH_DELEGATION_CONTEXT_KEY: delegation });
        let mut context = std::collections::HashMap::new();
        DelegatedTurnFacts::copy_metadata_to_context(Some(&metadata), &mut context);
        let facts = DelegatedTurnFacts::from_context(&context).unwrap();
        assert_eq!(
            facts.assistant_workspace.as_deref(),
            Some(Path::new(
                "/home/a/.openbitfun/personal_assistant/workspace"
            ))
        );
        assert_eq!(facts.trust, DelegationTrust::Normal);
        assert!(!facts.includes_private_context());

        // A turn without the delegation marker has no facts, and a missing or
        // unknown trust reads as normal.
        let mut plain = std::collections::HashMap::new();
        DelegatedTurnFacts::copy_metadata_to_context(Some(&json!({})), &mut plain);
        assert!(DelegatedTurnFacts::from_context(&plain).is_none());
        context.insert(
            DELEGATION_TRUST_CONTEXT_KEY.to_string(),
            "owner".to_string(),
        );
        assert_eq!(
            DelegatedTurnFacts::from_context(&context).unwrap().trust,
            DelegationTrust::Normal
        );
        context.insert(DELEGATION_TRUST_CONTEXT_KEY.to_string(), "full".to_string());
        assert!(DelegatedTurnFacts::from_context(&context)
            .unwrap()
            .includes_private_context());
    }

    #[cfg(feature = "agent-runtime")]
    #[test]
    fn assistant_tasks_write_only_in_their_own_directory() {
        let facts = DelegatedTurnFacts {
            assistant_workspace: Some(PathBuf::from("/a/personal_assistant/workspace")),
            task_dir: Some(PathBuf::from("/a/delegation/workspaces/job")),
            trust: DelegationTrust::Full,
        };
        let restricted = facts.restrict_tools(Default::default());
        for roots in [
            &restricted.path_policy.write_roots,
            &restricted.path_policy.edit_roots,
            &restricted.path_policy.delete_roots,
        ] {
            assert_eq!(roots, &vec!["/a/delegation/workspaces/job".to_string()]);
        }
        assert!(restricted.path_policy.read_roots.is_empty());

        // A delegated turn that does not run as an assistant keeps its tools.
        let plain = DelegatedTurnFacts {
            assistant_workspace: None,
            ..facts
        };
        assert!(plain
            .restrict_tools(Default::default())
            .path_policy
            .write_roots
            .is_empty());
    }

    #[tokio::test]
    async fn unreadable_entries_are_kept_closed_and_reported() {
        let (store, _dir) = store();
        tokio::fs::create_dir_all(store.path().parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            store.path(),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "entries": [
                    { "id": "future", "agentType": "agentic", "displayName": "Future",
                      "taskKinds": ["telepathy"], "sameAccountDevices": ["*"] },
                    { "id": "broken", "taskKinds": "message" },
                    { "id": "helper", "agentType": "agentic", "displayName": "Helper",
                      "taskKinds": ["message", "telepathy"], "sameAccountDevices": ["*"],
                      "addedLater": true }
                ]
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let records = store.records().await.unwrap();
        assert_eq!(records.len(), 3);
        assert!(records[0].agent.is_none() && records[0].problem.is_some());
        assert!(records[1].agent.is_none() && records[1].problem.is_some());
        // Legacy agent ids read back canonical.
        assert_eq!(records[2].agent.as_ref().unwrap().agent_type, "Standard");
        let granted: Vec<_> = store
            .granted_to_device("laptop")
            .await
            .unwrap()
            .into_iter()
            .map(|agent| agent.id)
            .collect();
        assert_eq!(granted, vec!["helper"]);

        // Editing one entry keeps the unreadable ones on disk.
        store.open(agent("another", &["laptop"])).await.unwrap();
        assert_eq!(store.records().await.unwrap().len(), 4);
        // An unreadable entry can still be closed by id.
        assert!(store.close("broken").await.unwrap());
        assert_eq!(store.records().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn an_unparseable_file_is_never_overwritten() {
        let (store, _dir) = store();
        tokio::fs::create_dir_all(store.path().parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(store.path(), b"{ not json").await.unwrap();
        assert!(store.records().await.is_err());
        assert!(store.open(agent("helper", &["laptop"])).await.is_err());
        assert_eq!(tokio::fs::read(store.path()).await.unwrap(), b"{ not json");
    }
}
