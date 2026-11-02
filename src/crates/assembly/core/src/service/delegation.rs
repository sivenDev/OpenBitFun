//! Delegated tasks: tasks another device gives one of this device's shared
//! assistants. Each runs as the assistant, in the tenant workspace of the
//! device that asked (see [`crate::service::lan_share`]), and cannot change
//! the assistant itself.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::infrastructure::PathManager;

const MAX_ASSISTANT_ID_LEN: usize = 64;
/// Turn context key a target sets on a delegated job's turns, naming the
/// device that asked. A turn that carries it must not delegate onward.
pub const DISPATCH_DELEGATION_CONTEXT_KEY: &str = "dispatchDelegation";
/// Job ids a requester gives the jobs it delegates. Its manual dispatch list
/// never adopts them: the delegating subagent already observes them.
pub const DELEGATED_JOB_ID_PREFIX: &str = "delegation-";

/// Agent type of an assistant: `assistant:<directory>` names one directory
/// under this device's assistant workspace base.
pub const ASSISTANT_AGENT_TYPE_PREFIX: &str = "assistant:";
/// The agent mode a shared assistant's tasks run in.
pub const ASSISTANT_RUN_AGENT_TYPE: &str = "Claw";
const DEFAULT_ASSISTANT_DIRECTORY: &str = "workspace";
const NAMED_ASSISTANT_DIRECTORY_PREFIX: &str = "workspace-";

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
                    && id.len() <= MAX_ASSISTANT_ID_LEN
                    && id
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            });
    valid.then_some(directory)
}

/// The agent type of the assistant in `directory`.
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

/// An assistant on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAssistant {
    /// Its agent type (`assistant:<directory>`).
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

/// Whether `path` is a tenant workspace a delegated task ran in. Such
/// sessions belong to another device's user and never feed the owner's
/// memories.
pub fn is_delegation_workspace_path(path: &Path) -> bool {
    crate::infrastructure::try_get_path_manager_arc()
        .is_ok_and(|path_manager| crate::service::lan_share::is_tenant_path(&path_manager, path))
}

/// How far the owner trusts the users of a shared assistant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationTrust {
    /// The default: the task cannot reach the owner's desktop, other
    /// sessions, schedules or publishing, and gets no owner memory.
    #[default]
    Normal,
    /// The owner chose to trust the users fully: the task runs like the
    /// owner's own.
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

/// Tools a delegated task at normal trust cannot use: they reach the owner's
/// desktop, other sessions, schedules, goals, publishing or this app's own
/// controls. `SessionHistory` and `SessionControl` are not here: they serve
/// the task's own tenant workspace and refuse everything else themselves
/// (see [`DelegatedTurnFacts::session_workspace_refusal`]).
pub const NORMAL_TRUST_DENIED_TOOLS: [&str; 11] = [
    "SessionMessage",
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

const NORMAL_TRUST_REFUSAL: &str = "is not available to a task from a device this assistant is \
     shared with: the owner shares it at normal trust";

/// Turn context keys that carry the facts of a delegated turn. The target's
/// worker writes them into the `dispatchDelegation` turn metadata; the kernel
/// copies them into the turn and every subagent it starts.
pub const DELEGATION_ASSISTANT_CONTEXT_KEY: &str = "dispatchDelegationAssistant";
pub const DELEGATION_TENANT_CONTEXT_KEY: &str = "dispatchDelegationTenant";
pub const DELEGATION_TRUST_CONTEXT_KEY: &str = "dispatchDelegationTrust";
const METADATA_ASSISTANT_FIELD: &str = "assistantWorkspace";
const METADATA_TENANT_FIELD: &str = "tenantWorkspace";
const METADATA_TRUST_FIELD: &str = "trust";

/// What the kernel knows about a turn another device delegated here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedTurnFacts {
    /// Root of the shared assistant the task runs as; read-only to it.
    pub assistant_workspace: Option<PathBuf>,
    /// The tenant workspace of the device that asked: the only place the
    /// task writes, where that device's sessions are kept.
    pub tenant_workspace: Option<PathBuf>,
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
            (METADATA_TENANT_FIELD, DELEGATION_TENANT_CONTEXT_KEY),
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
        tenant_workspace: &str,
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
            METADATA_TENANT_FIELD.to_string(),
            Value::String(tenant_workspace.to_string()),
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
            tenant_workspace: path(DELEGATION_TENANT_CONTEXT_KEY),
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
        DELEGATION_TENANT_CONTEXT_KEY,
        DELEGATION_TRUST_CONTEXT_KEY,
    ];

    /// Whether the task may see the owner's memory summary. The owner's
    /// `USER.md` never enters a delegated task's prompt: its user profile is
    /// the tenant's own.
    pub fn includes_private_context(&self) -> bool {
        self.trust == DelegationTrust::Full
    }

    /// Why a session tool of this task cannot reach the sessions of
    /// `workspace`, if it cannot. At normal trust a task reaches only its
    /// tenant workspace; at full trust, every workspace.
    pub fn session_workspace_refusal(&self, tool: &str, workspace: &str) -> Option<String> {
        let scope = match self.trust {
            DelegationTrust::Full => return None,
            DelegationTrust::Normal => self.tenant_workspace.as_deref(),
        };
        let normalize = |path: &str| path.replace('\\', "/").trim_end_matches('/').to_string();
        let allowed =
            scope.is_some_and(|scope| normalize(&scope.to_string_lossy()) == normalize(workspace));
        (!allowed).then(|| {
            format!("{tool} reaches only the conversations of the device this task came from")
        })
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
    /// assistant writes only in its tenant workspace, so it can never change
    /// the assistant's persona, instructions or memory. At normal trust the
    /// tools in [`NORMAL_TRUST_DENIED_TOOLS`] are refused as well.
    #[cfg(feature = "agent-runtime")]
    pub fn restrict_tools(
        &self,
        mut restrictions: crate::agentic::tools::ToolRuntimeRestrictions,
    ) -> crate::agentic::tools::ToolRuntimeRestrictions {
        if let (Some(_), Some(tenant)) = (&self.assistant_workspace, &self.tenant_workspace) {
            let root = tenant.to_string_lossy().into_owned();
            let policy = &mut restrictions.path_policy;
            for roots in [
                &mut policy.write_roots,
                &mut policy.edit_roots,
                &mut policy.delete_roots,
            ] {
                if roots.is_empty() {
                    roots.push(root.clone());
                } else {
                    roots.retain(|existing| Path::new(existing).starts_with(tenant));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn delegated_turn_facts_travel_from_metadata_to_context() {
        let fields = DelegatedTurnFacts::metadata_fields(
            Some("/home/a/.openbitfun/personal_assistant/workspace"),
            "/home/a/.openbitfun/delegation/tenants/workspace/0123",
            DelegationTrust::Normal,
        );
        let mut delegation = json!({ "assistantId": "workspace" });
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
            tenant_workspace: Some(PathBuf::from("/a/delegation/tenants/workspace/0123")),
            trust: DelegationTrust::Full,
        };
        let restricted = facts.restrict_tools(Default::default());
        for roots in [
            &restricted.path_policy.write_roots,
            &restricted.path_policy.edit_roots,
            &restricted.path_policy.delete_roots,
        ] {
            assert_eq!(
                roots,
                &vec!["/a/delegation/tenants/workspace/0123".to_string()]
            );
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

    fn delegated_facts(trust: DelegationTrust) -> DelegatedTurnFacts {
        DelegatedTurnFacts {
            assistant_workspace: Some(PathBuf::from("/home/me/personal_assistant/workspace")),
            tenant_workspace: Some(PathBuf::from("/home/me/delegation/tenants/workspace/0123")),
            trust,
        }
    }

    #[cfg(feature = "agent-runtime")]
    #[test]
    fn normal_trust_refuses_the_owners_desktop_sessions_and_publishing() {
        let normal = delegated_facts(DelegationTrust::Normal).restrict_tools(Default::default());
        for tool in NORMAL_TRUST_DENIED_TOOLS {
            assert!(normal.denied_tool_names.contains(tool), "{tool}");
            assert!(normal.denied_tool_messages[tool].contains("shares it at normal trust"));
        }
        let full = delegated_facts(DelegationTrust::Full).restrict_tools(Default::default());
        assert!(full.denied_tool_names.is_empty());
        // Either way the task writes only in its own directory.
        for restrictions in [&normal, &full] {
            assert_eq!(
                restrictions.path_policy.write_roots,
                vec!["/home/me/delegation/tenants/workspace/0123".to_string()]
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
    fn normal_trust_reaches_only_the_sessions_of_its_tenant() {
        let normal = delegated_facts(DelegationTrust::Normal);
        let tenant = "/home/me/delegation/tenants/workspace/0123";
        assert_eq!(
            normal.session_workspace_refusal("SessionHistory", tenant),
            None
        );
        assert_eq!(
            normal.session_workspace_refusal("SessionHistory", &format!("{tenant}/")),
            None
        );
        for other in [
            "/home/me/personal_assistant/workspace",
            "/home/me/delegation/tenants/workspace/4567",
            "/home/me/delegation/tenants/workspace",
        ] {
            let refusal = normal
                .session_workspace_refusal("SessionControl", other)
                .expect(other);
            assert!(
                refusal.starts_with("SessionControl reaches only"),
                "{refusal}"
            );
        }
        let full = delegated_facts(DelegationTrust::Full);
        assert_eq!(
            full.session_workspace_refusal("SessionHistory", "/home/me/elsewhere"),
            None
        );
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
