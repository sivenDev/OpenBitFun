//! Assistants this device opens to other devices for delegated dispatch jobs.
//!
//! Only assistants are opened. Each delegated task is a message task that runs
//! as the assistant in a directory of its own.
//!
//! The open list is device-local: it lives beside the dispatch data rather than
//! in the shared configuration document, so a configuration backup restored on
//! another device never opens agents there. A list this build cannot read is
//! reported and never overwritten.

use std::path::{Path, PathBuf};

use openbitfun_services_core::json_store::{JsonFileStore, JsonFileStoreError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::infrastructure::PathManager;

const DELEGATION_DIR: &str = "delegation";
const OPEN_AGENTS_FILE: &str = "open-agents.json";
const CONNECTION_TRUST_FILE: &str = "connection-trust.json";
const CONNECTION_TRUST_SCHEMA_VERSION: u32 = 1;
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
/// directory under this device's assistant workspace base.
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

/// Tools a task another account delegated at normal trust cannot use: they
/// reach the owner's desktop, other sessions, schedules, goals, publishing
/// or this app's own controls. The single list every check reads.
pub const NORMAL_TRUST_DENIED_TOOLS: [&str; 13] = [
    "SessionHistory",
    "SessionMessage",
    "SessionControl",
    "ListWorkspaces",
    "ComputerUse",
    "ControlHub",
    "Cron",
    "create_goal",
    "update_goal",
    "PageDeploy",
    "PagePublish",
    "PublishAppearance",
    "OpenBitFunControl",
];

/// Subagent types built around the tools above; a normal-trust task cannot
/// start them.
pub const NORMAL_TRUST_DENIED_SUBAGENTS: [&str; 2] = ["ComputerUse", "OpenBitFun"];

const NORMAL_TRUST_REFUSAL: &str = "is not available to a task another account delegated here: \
     the owner of this device has not trusted this connection fully";

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

    /// The facts of a delegated turn from a tool context's custom data, or
    /// `None` for any other turn.
    pub fn from_tool_data(data: &std::collections::HashMap<String, Value>) -> Option<Self> {
        let mut context = std::collections::HashMap::new();
        for key in
            std::iter::once(DISPATCH_DELEGATION_CONTEXT_KEY).chain(Self::INHERITED_CONTEXT_KEYS)
        {
            let value = match data.get(key) {
                Some(Value::Bool(value)) => value.to_string(),
                Some(Value::String(value)) => value.clone(),
                _ => continue,
            };
            context.insert(key.to_string(), value);
        }
        Self::from_context(&context)
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

    /// Why this task cannot start a subagent of `agent_type`, if it cannot.
    pub fn subagent_refusal(&self, agent_type: &str) -> Option<String> {
        (self.trust == DelegationTrust::Normal
            && NORMAL_TRUST_DENIED_SUBAGENTS.contains(&agent_type))
        .then(|| format!("agent_type {agent_type} {NORMAL_TRUST_REFUSAL}"))
    }

    /// What a normal-trust task reads only with the owner's approval each
    /// time: the assistant's `USER.md` and the memory directories. Empty at
    /// full trust.
    pub fn owner_private_paths(&self) -> Vec<PathBuf> {
        if self.includes_private_context() {
            return Vec::new();
        }
        let mut paths = Vec::new();
        if let Some(assistant) = &self.assistant_workspace {
            paths.push(assistant.join("USER.md"));
        }
        if let Ok(path_manager) = crate::infrastructure::try_get_path_manager_arc() {
            paths.push(path_manager.memories_root_dir());
            if let Some(database_dir) = path_manager.memories_database_file().parent() {
                paths.push(database_dir.to_path_buf());
            }
        }
        paths
    }

    /// Narrows `restrictions` for this turn: a task that runs as an
    /// assistant writes only in its own directory, so it can never change
    /// the assistant's persona, instructions or memory. At normal trust the
    /// tools in [`NORMAL_TRUST_DENIED_TOOLS`] are refused as well.
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
        if self.trust == DelegationTrust::Normal {
            for tool in NORMAL_TRUST_DENIED_TOOLS {
                restrictions.denied_tool_names.insert(tool.to_string());
                restrictions
                    .denied_tool_messages
                    .insert(tool.to_string(), format!("{tool} {NORMAL_TRUST_REFUSAL}"));
            }
        }
        restrictions
    }
}

/// One assistant this device opens to other devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAgent {
    /// Stable id the requester addresses.
    pub id: String,
    /// The opened assistant, `assistant:<directory>`.
    #[serde(deserialize_with = "openbitfun_core_types::agent_identity::deserialize_agent_id")]
    pub agent_type: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
    /// Tasks that may run at once; more wait, queued, until one finishes.
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
        if assistant_directory(agent_type).is_none() {
            return Err(format!("'{agent_type}' does not name an assistant"));
        }
        if self.display_name.trim().is_empty() {
            return Err("open agent requires a display name".to_string());
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenAgentsDocument {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    entries: Vec<OpenAgent>,
}

/// Why a delegated job is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DelegationRefusal {
    #[error("open agent '{0}' does not exist on this device")]
    UnknownOpenAgent(String),
    #[error("open agent '{0}' is not granted to the requesting device")]
    NotGranted(String),
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

    /// Every opened assistant.
    pub async fn agents(&self) -> Result<Vec<OpenAgent>, OpenAgentStoreError> {
        let document: OpenAgentsDocument = JsonFileStore
            .read_locked_optional(&self.path)
            .await?
            .unwrap_or_default();
        for agent in &document.entries {
            agent.validate().map_err(|problem| {
                OpenAgentStoreError::Invalid(format!("open agent '{}': {problem}", agent.id))
            })?;
        }
        Ok(document.entries)
    }

    /// The entry `open_agent_id`, or why it cannot take a task.
    async fn open_agent(
        &self,
        open_agent_id: &str,
    ) -> Result<Result<OpenAgent, DelegationRefusal>, OpenAgentStoreError> {
        Ok(self
            .agents()
            .await?
            .into_iter()
            .find(|agent| agent.id == open_agent_id)
            .ok_or_else(|| DelegationRefusal::UnknownOpenAgent(open_agent_id.to_string())))
    }

    /// Entries granted to `device_id`.
    pub async fn granted_to_device(
        &self,
        device_id: &str,
    ) -> Result<Vec<OpenAgent>, OpenAgentStoreError> {
        Ok(self
            .agents()
            .await?
            .into_iter()
            .filter(|agent| agent.is_granted_to_device(device_id))
            .collect())
    }

    /// The open agent a delegated job from `device_id` may run, or why not.
    pub async fn authorize(
        &self,
        device_id: &str,
        open_agent_id: &str,
    ) -> Result<Result<OpenAgent, DelegationRefusal>, OpenAgentStoreError> {
        if device_id.trim().is_empty() {
            return Ok(Err(DelegationRefusal::MissingRequester));
        }
        let agent = match self.open_agent(open_agent_id).await? {
            Ok(agent) => agent,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if !agent.is_granted_to_device(device_id) {
            return Ok(Err(DelegationRefusal::NotGranted(agent.id)));
        }
        Ok(Ok(agent))
    }

    /// Entries granted to the cross-account connection.
    pub async fn granted_to_connection(
        &self,
        connection_id: &str,
    ) -> Result<Vec<OpenAgent>, OpenAgentStoreError> {
        Ok(self
            .agents()
            .await?
            .into_iter()
            .filter(|agent| agent.is_granted_to_connection(connection_id))
            .collect())
    }

    /// The open agent a delegated job over `connection_id` may run, or why
    /// not. Same-account device grants never apply to a connection.
    pub async fn authorize_connection(
        &self,
        connection_id: &str,
        open_agent_id: &str,
    ) -> Result<Result<OpenAgent, DelegationRefusal>, OpenAgentStoreError> {
        if connection_id.trim().is_empty() {
            return Ok(Err(DelegationRefusal::MissingRequester));
        }
        let agent = match self.open_agent(open_agent_id).await? {
            Ok(agent) => agent,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if !agent.is_granted_to_connection(connection_id) {
            return Ok(Err(DelegationRefusal::NotGrantedToConnection(agent.id)));
        }
        Ok(Ok(agent))
    }

    /// Grants the open agent `open_agent_id` to a cross-account connection.
    pub async fn grant_connection(
        &self,
        open_agent_id: &str,
        connection_id: &str,
    ) -> Result<OpenAgent, OpenAgentStoreError> {
        let connection_id = connection_id.trim();
        if connection_id.is_empty() {
            return Err(OpenAgentStoreError::Invalid(
                "a connection id is required".to_string(),
            ));
        }
        let mut agent = self.readable(open_agent_id).await?;
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
        self.agents()
            .await?
            .into_iter()
            .find(|agent| agent.id == open_agent_id)
            .ok_or_else(|| {
                OpenAgentStoreError::Invalid(format!(
                    "open agent '{open_agent_id}' does not exist on this device"
                ))
            })
    }

    /// Stores an opened agent. Connection grants are not part of opening:
    /// the stored entry's grants carry over while it runs the same agent.
    async fn store_entry(&self, mut agent: OpenAgent) -> Result<(), OpenAgentStoreError> {
        agent.agent_type =
            openbitfun_core_types::agent_identity::canonical_agent_id(agent.agent_type.trim())
                .to_string();
        agent.connections = self
            .agents()
            .await?
            .into_iter()
            .find(|stored| stored.id == agent.id && stored.agent_type == agent.agent_type)
            .map(|stored| stored.connections)
            .unwrap_or_default();
        self.write_entry(agent).await
    }

    async fn write_entry(&self, agent: OpenAgent) -> Result<(), OpenAgentStoreError> {
        agent.validate().map_err(OpenAgentStoreError::Invalid)?;
        JsonFileStore
            .update_locked(
                &self.path,
                OpenAgentsDocument::default(),
                |document: &mut OpenAgentsDocument| {
                    document.schema_version = OPEN_AGENTS_SCHEMA_VERSION;
                    match document
                        .entries
                        .iter_mut()
                        .find(|entry| entry.id == agent.id)
                    {
                        Some(entry) => *entry = agent,
                        None => document.entries.push(agent),
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
        if agent.display_name.trim().is_empty() {
            agent.display_name = assistant_directory(&agent.agent_type)
                .unwrap_or_default()
                .to_string();
        }
        self.store_entry(agent.clone()).await?;
        Ok(agent)
    }

    /// Closes the entry with `id`. Returns whether one existed.
    pub async fn close(&self, id: &str) -> Result<bool, OpenAgentStoreError> {
        let (removed, _) = JsonFileStore
            .update_locked(
                &self.path,
                OpenAgentsDocument::default(),
                |document: &mut OpenAgentsDocument| {
                    let before = document.entries.len();
                    document.entries.retain(|entry| entry.id != id);
                    before != document.entries.len()
                },
            )
            .await?;
        Ok(removed)
    }
}

/// How far this device's owner trusts each cross-account connection. A
/// connection without an entry is trusted normally.
#[derive(Debug, Clone)]
pub struct ConnectionTrustStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionTrustDocument {
    #[serde(default)]
    schema_version: u32,
    /// Connection id to level; only levels other than normal are stored.
    #[serde(default)]
    connections: std::collections::BTreeMap<String, DelegationTrust>,
}

impl ConnectionTrustStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_product_home(path_manager: &PathManager) -> Self {
        Self::new(
            path_manager
                .product_home_dir()
                .join(DELEGATION_DIR)
                .join(CONNECTION_TRUST_FILE),
        )
    }

    /// Every connection trusted beyond normal.
    pub async fn levels(
        &self,
    ) -> Result<std::collections::BTreeMap<String, DelegationTrust>, OpenAgentStoreError> {
        let document: ConnectionTrustDocument = JsonFileStore
            .read_locked_optional(&self.path)
            .await?
            .unwrap_or_default();
        Ok(document
            .connections
            .into_iter()
            .filter(|(_, level)| *level != DelegationTrust::Normal)
            .collect())
    }

    /// The level of `connection_id`. A store that cannot be read trusts
    /// normally, the safer side, and says why in the log.
    pub async fn level(&self, connection_id: &str) -> DelegationTrust {
        match self.levels().await {
            Ok(levels) => levels.get(connection_id).copied().unwrap_or_default(),
            Err(error) => {
                log::warn!(
                    "Connection trust store unreadable, trusting connection normally: path={}, error={}",
                    self.path.display(),
                    error
                );
                DelegationTrust::Normal
            }
        }
    }

    /// Sets the level of `connection_id`; normal removes its entry.
    pub async fn set(
        &self,
        connection_id: &str,
        level: DelegationTrust,
    ) -> Result<(), OpenAgentStoreError> {
        let connection_id = connection_id.trim();
        if connection_id.is_empty() {
            return Err(OpenAgentStoreError::Invalid(
                "a connection id is required".to_string(),
            ));
        }
        JsonFileStore
            .update_locked(
                &self.path,
                ConnectionTrustDocument::default(),
                |document: &mut ConnectionTrustDocument| {
                    document.schema_version = CONNECTION_TRUST_SCHEMA_VERSION;
                    match level {
                        DelegationTrust::Normal => {
                            document.connections.remove(connection_id);
                        }
                        level => {
                            document
                                .connections
                                .insert(connection_id.to_string(), level);
                        }
                    }
                },
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(id: &str, devices: &[&str]) -> OpenAgent {
        OpenAgent {
            id: id.to_string(),
            agent_type: "assistant:workspace".to_string(),
            display_name: "Helper".to_string(),
            description: None,
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
        assert!(store.agents().await.unwrap().is_empty());
        assert!(store.granted_to_device("laptop").await.unwrap().is_empty());
        assert_eq!(
            store.authorize("laptop", "helper").await.unwrap(),
            Err(DelegationRefusal::UnknownOpenAgent("helper".to_string()))
        );
    }

    #[tokio::test]
    async fn grants_are_per_device() {
        let (store, _dir) = store();
        store
            .store_entry(agent("helper", &["laptop"]))
            .await
            .unwrap();
        store
            .store_entry(agent("everyone", &[ALL_SAME_ACCOUNT_DEVICES]))
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

        assert!(store.authorize("laptop", "helper").await.unwrap().is_ok());
        assert_eq!(
            store.authorize("phone", "helper").await.unwrap(),
            Err(DelegationRefusal::NotGranted("helper".to_string()))
        );
        assert_eq!(
            store.authorize("", "everyone").await.unwrap(),
            Err(DelegationRefusal::MissingRequester)
        );
    }

    #[tokio::test]
    async fn connections_are_granted_one_by_one_and_apart_from_devices() {
        let (store, _dir) = store();
        store.store_entry(agent("helper", &["*"])).await.unwrap();
        store.store_entry(agent("reviewer", &[])).await.unwrap();

        // Nothing is granted to a connection by default, not even "*".
        assert!(store
            .granted_to_connection("conn-1")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .authorize_connection("conn-1", "helper")
                .await
                .unwrap(),
            Err(DelegationRefusal::NotGrantedToConnection(
                "helper".to_string()
            ))
        );

        store.grant_connection("reviewer", "conn-1").await.unwrap();
        let granted = store.granted_to_connection("conn-1").await.unwrap();
        assert_eq!(granted.len(), 1);
        assert_eq!(granted[0].id, "reviewer");
        assert!(store
            .granted_to_connection("conn-2")
            .await
            .unwrap()
            .is_empty());
        assert!(store
            .authorize_connection("conn-1", "reviewer")
            .await
            .unwrap()
            .is_ok());
        // A connection grant never admits a same-account device, and the
        // device id "conn-1" is not the connection.
        assert_eq!(
            store.authorize("conn-1", "reviewer").await.unwrap(),
            Err(DelegationRefusal::NotGranted("reviewer".to_string()))
        );

        // Granting twice keeps one grant; revoking removes it.
        store.grant_connection("reviewer", "conn-1").await.unwrap();
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
        assert!(store.grant_connection("missing", "conn-1").await.is_err());
        assert!(store.grant_connection("reviewer", " ").await.is_err());
    }

    #[tokio::test]
    async fn connection_limits_are_set_per_grant_and_kept_on_regrant() {
        let (store, _dir) = store();
        store.store_entry(agent("helper", &[])).await.unwrap();
        let limits = ConnectionLimits {
            max_concurrent_jobs: Some(2),
            max_job_minutes: Some(30),
        };
        // Limits apply only to an existing grant.
        assert!(store
            .set_connection_limits("helper", "conn-1", limits)
            .await
            .is_err());
        store.grant_connection("helper", "conn-1").await.unwrap();
        store
            .set_connection_limits("helper", "conn-1", limits)
            .await
            .unwrap();
        // Granting again keeps them.
        let agent = store.grant_connection("helper", "conn-1").await.unwrap();
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
        // Stored flat beside the connection id.
        let raw = std::fs::read_to_string(store.path()).unwrap();
        assert!(raw.contains("\"maxConcurrentJobs\":2"), "{raw}");
    }

    #[tokio::test]
    async fn reopening_keeps_connection_grants_unless_the_agent_changes() {
        let (store, _dir) = store();
        store.store_entry(agent("helper", &[])).await.unwrap();
        store.grant_connection("helper", "conn-1").await.unwrap();

        let mut edited = agent("helper", &["laptop"]);
        edited.display_name = "Renamed".to_string();
        store.store_entry(edited).await.unwrap();
        assert_eq!(
            store.granted_to_connection("conn-1").await.unwrap().len(),
            1
        );

        let mut other = agent("helper", &[]);
        other.agent_type = "assistant:workspace-other".to_string();
        store.store_entry(other).await.unwrap();
        assert!(store
            .granted_to_connection("conn-1")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn reopening_replaces_and_closing_removes() {
        let (store, _dir) = store();
        store
            .store_entry(agent("helper", &["laptop"]))
            .await
            .unwrap();
        store
            .store_entry(agent("helper", &["phone"]))
            .await
            .unwrap();
        let agents = store.agents().await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].same_account_devices, vec!["phone"]);
        assert_eq!(agents[0].agent_type, "assistant:workspace");
        assert!(store.close("helper").await.unwrap());
        assert!(!store.close("helper").await.unwrap());
        assert!(store.agents().await.unwrap().is_empty());
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
                agent_type: "Standard".to_string(),
                ..agent("x", &["laptop"])
            },
            OpenAgent {
                agent_type: "acp:codex".to_string(),
                ..agent("x", &["laptop"])
            },
            OpenAgent {
                agent_type: "assistant:notes".to_string(),
                ..agent("x", &["laptop"])
            },
            OpenAgent {
                agent_type: "remote:device/helper".to_string(),
                ..agent("x", &["laptop"])
            },
        ] {
            assert!(matches!(
                store.store_entry(invalid).await,
                Err(OpenAgentStoreError::Invalid(_))
            ));
        }
        assert!(store.agents().await.unwrap().is_empty());
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
    async fn assistants_open_only_with_confirmation() {
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
        let opened = store
            .open_assistant(&base, entry.clone(), true)
            .await
            .unwrap();
        assert_eq!(opened.display_name, "workspace-ann");

        let granted = store.authorize("laptop", "ann").await.unwrap().unwrap();
        assert_eq!(granted.agent_type, "assistant:workspace-ann");
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
    async fn an_unparseable_file_is_never_overwritten() {
        let (store, _dir) = store();
        tokio::fs::create_dir_all(store.path().parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(store.path(), b"{ not json").await.unwrap();
        assert!(store.agents().await.is_err());
        assert!(store
            .store_entry(agent("helper", &["laptop"]))
            .await
            .is_err());
        assert_eq!(tokio::fs::read(store.path()).await.unwrap(), b"{ not json");
    }

    #[tokio::test]
    async fn connections_are_trusted_normally_until_the_owner_trusts_them_fully() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionTrustStore::new(dir.path().join("connection-trust.json"));
        assert_eq!(store.level("conn-1").await, DelegationTrust::Normal);

        store.set("conn-1", DelegationTrust::Full).await.unwrap();
        assert_eq!(store.level("conn-1").await, DelegationTrust::Full);
        assert_eq!(store.level("conn-2").await, DelegationTrust::Normal);

        store.set("conn-1", DelegationTrust::Normal).await.unwrap();
        assert_eq!(store.level("conn-1").await, DelegationTrust::Normal);
        assert!(store.levels().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unreadable_trust_reads_as_normal_and_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connection-trust.json");
        let store = ConnectionTrustStore::new(path.clone());
        store.set("conn-2", DelegationTrust::Full).await.unwrap();
        assert_eq!(store.level("conn-2").await, DelegationTrust::Full);

        tokio::fs::write(&path, b"{ not json").await.unwrap();
        assert_eq!(store.level("conn-2").await, DelegationTrust::Normal);
        assert!(store.set("conn-2", DelegationTrust::Full).await.is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"{ not json");
    }

    fn delegated_facts(trust: DelegationTrust) -> DelegatedTurnFacts {
        DelegatedTurnFacts {
            assistant_workspace: Some(PathBuf::from("/home/me/personal_assistant/workspace")),
            task_dir: Some(PathBuf::from("/home/me/delegation/workspaces/job")),
            trust,
        }
    }

    #[cfg(feature = "agent-runtime")]
    #[test]
    fn normal_trust_refuses_the_owners_desktop_sessions_and_publishing() {
        let normal = delegated_facts(DelegationTrust::Normal).restrict_tools(Default::default());
        for tool in NORMAL_TRUST_DENIED_TOOLS {
            assert!(normal.denied_tool_names.contains(tool), "{tool}");
            assert!(normal.denied_tool_messages[tool].contains("not trusted this connection fully"));
        }
        let full = delegated_facts(DelegationTrust::Full).restrict_tools(Default::default());
        assert!(full.denied_tool_names.is_empty());
        // Either way the task writes only in its own directory.
        for restrictions in [&normal, &full] {
            assert_eq!(
                restrictions.path_policy.write_roots,
                vec!["/home/me/delegation/workspaces/job".to_string()]
            );
        }
    }

    #[test]
    fn normal_trust_refuses_desktop_subagents_and_guards_private_files() {
        let normal = delegated_facts(DelegationTrust::Normal);
        assert!(normal.subagent_refusal("ComputerUse").is_some());
        assert!(normal.subagent_refusal("OpenBitFun").is_some());
        assert!(normal.subagent_refusal("Explore").is_none());
        assert!(normal.owner_private_paths().contains(&PathBuf::from(
            "/home/me/personal_assistant/workspace/USER.md"
        )));

        let full = delegated_facts(DelegationTrust::Full);
        assert!(full.subagent_refusal("ComputerUse").is_none());
        assert!(full.owner_private_paths().is_empty());
    }

    #[test]
    fn tool_data_carries_the_delegated_turn_facts() {
        let mut data = std::collections::HashMap::new();
        assert!(DelegatedTurnFacts::from_tool_data(&data).is_none());
        data.insert(
            DISPATCH_DELEGATION_CONTEXT_KEY.to_string(),
            Value::Bool(true),
        );
        data.insert(
            DELEGATION_TRUST_CONTEXT_KEY.to_string(),
            Value::String("full".to_string()),
        );
        let facts = DelegatedTurnFacts::from_tool_data(&data).unwrap();
        assert_eq!(facts.trust, DelegationTrust::Full);
        data.remove(DELEGATION_TRUST_CONTEXT_KEY);
        assert_eq!(
            DelegatedTurnFacts::from_tool_data(&data).unwrap().trust,
            DelegationTrust::Normal
        );
    }
}
