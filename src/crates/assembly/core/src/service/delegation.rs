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
        // Delegated jobs never reach another device, and external agents are
        // opened by a later capability.
        if agent_type.starts_with("acp:") || agent_type.starts_with("remote:") {
            return Err(format!(
                "agent type '{agent_type}' cannot be opened to other devices"
            ));
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
        Ok(())
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

    /// Opens `agent`, replacing any stored entry with the same id.
    pub async fn open(&self, mut agent: OpenAgent) -> Result<(), OpenAgentStoreError> {
        agent.agent_type =
            openbitfun_core_types::agent_identity::canonical_agent_id(agent.agent_type.trim())
                .to_string();
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
                agent_type: "acp:claude-code".to_string(),
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
