use crate::agentic::agents::{
    get_agent_registry, render_direct_tool_listing_body, PromptBuilder, SubagentListScope,
    SubagentQueryContext, ToolListingSections, UserContextPolicy,
};
use crate::agentic::tools::framework::ToolUseContext;
use crate::agentic::tools::implementations::skills::{get_skill_registry, SkillInfo};
use crate::agentic::tools::implementations::task::{AgentExecutionTool, ExternalAgentCatalog};
use crate::agentic::tools::manifest_resolver::{resolve_tool_manifest, ResolvedToolManifest};
use crate::agentic::tools::product_runtime::GetToolSpecTool;
use crate::agentic::tools::tool_context_runtime;
use crate::agentic::tools::ToolRuntimeRestrictions;
use crate::agentic::workspace::WorkspaceServices;
use crate::agentic::WorkspaceBinding;
pub use openbitfun_agent_runtime::skill_agent_snapshot::{
    build_skill_agent_tool_listing_sections_from_snapshot, diff_skill_agent_snapshot,
    render_full_agent_listing_body, render_full_skill_listing_body, AgentSnapshotEntry,
    DeferredToolSnapshotEntry, SkillAgentDiff, SkillSnapshotEntry, TurnSkillAgentSnapshot,
    TurnSkillAgentSnapshotStore,
};

#[derive(Debug, Clone)]
pub struct SkillAgentSnapshotResolution {
    pub snapshot: TurnSkillAgentSnapshot,
    pub tool_listing_sections: ToolListingSections,
}

pub async fn resolve_skill_agent_snapshot(
    agent_type: &str,
    workspace: Option<&WorkspaceBinding>,
    workspace_services: Option<&WorkspaceServices>,
    enable_tools: bool,
    context_vars: &std::collections::HashMap<String, String>,
    runtime_tool_restrictions: &ToolRuntimeRestrictions,
) -> SkillAgentSnapshotResolution {
    if !enable_tools {
        return SkillAgentSnapshotResolution {
            snapshot: TurnSkillAgentSnapshot {
                deferred_tools: Some(Vec::new()),
                ..Default::default()
            },
            tool_listing_sections: ToolListingSections::default(),
        };
    }

    let agent_registry = get_agent_registry();
    agent_registry
        .load_custom_agents(
            workspace
                .filter(|binding| !binding.is_remote())
                .and_then(|binding| binding.workspace_id.as_deref()),
        )
        .await;

    let tool_policy = agent_registry
        .get_agent_tool_policy(
            agent_type,
            workspace.and_then(|binding| binding.workspace_id.as_deref()),
        )
        .await;

    let tool_description_context = tool_context_runtime::build_tool_description_context(
        agent_type,
        workspace,
        workspace_services,
        None,
        None,
        None,
        None,
        context_vars,
        runtime_tool_restrictions,
    );
    let manifest = resolve_tool_manifest(
        &tool_policy.allowed_tools,
        &tool_policy.exposure_overrides,
        &tool_description_context,
    )
    .await;

    let snapshot = build_skill_agent_snapshot(
        workspace,
        workspace_services,
        agent_type,
        &manifest,
        runtime_tool_restrictions,
        &tool_description_context,
    )
    .await;
    let tool_listing_sections = build_tool_listing_sections(&manifest, &snapshot);

    SkillAgentSnapshotResolution {
        snapshot,
        tool_listing_sections,
    }
}

async fn build_skill_agent_snapshot(
    workspace: Option<&WorkspaceBinding>,
    workspace_services: Option<&WorkspaceServices>,
    agent_type: &str,
    manifest: &ResolvedToolManifest,
    runtime_tool_restrictions: &ToolRuntimeRestrictions,
    tool_context: &ToolUseContext,
) -> TurnSkillAgentSnapshot {
    let has_tool = |tool_name: &str| {
        manifest
            .tool_definitions
            .iter()
            .any(|definition| definition.name == tool_name)
    };

    let mut snapshot = TurnSkillAgentSnapshot::default();

    if has_tool("Skill") {
        snapshot.skills = load_skill_entries(
            workspace,
            workspace_services,
            Some(agent_type),
            has_tool("generate_image"),
        )
        .await;
    }

    if has_tool("Task") || has_tool("AgentSpawn") {
        snapshot.subagents = load_subagent_entries(
            workspace,
            tool_context,
            runtime_tool_restrictions,
            AgentExecutionTool::external_agent_catalog(tool_context)
                .await
                .as_ref(),
        )
        .await;
    }

    snapshot.deferred_tools = Some(deferred_tool_entries(manifest));

    snapshot
}

fn deferred_tool_entries(manifest: &ResolvedToolManifest) -> Vec<DeferredToolSnapshotEntry> {
    if !manifest
        .tool_definitions
        .iter()
        .any(|definition| definition.name == "GetToolSpec")
    {
        return Vec::new();
    }
    manifest
        .deferred_tool_summaries
        .iter()
        .map(DeferredToolSnapshotEntry::from)
        .collect()
}

fn build_tool_listing_sections(
    manifest: &ResolvedToolManifest,
    snapshot: &TurnSkillAgentSnapshot,
) -> ToolListingSections {
    let has_tool = |tool_name: &str| {
        manifest
            .tool_definitions
            .iter()
            .any(|definition| definition.name == tool_name)
    };

    ToolListingSections {
        skill_listing: has_tool("Skill")
            .then(|| render_full_skill_listing_body(&snapshot.skills))
            .filter(|body| !body.is_empty()),
        agent_listing: (has_tool("Task") || has_tool("AgentSpawn"))
            .then(|| render_full_agent_listing_body(&snapshot.subagents))
            .filter(|body| !body.is_empty()),
        direct_tool_listing: (!manifest.deferred_tool_names.is_empty()).then(|| {
            render_direct_tool_listing_body(
                manifest
                    .tool_definitions
                    .iter()
                    .map(|definition| definition.name.as_str()),
            )
        }),
        deferred_tool_listing: if has_tool("GetToolSpec") {
            GetToolSpecTool::build_deferred_tools_context_section(&manifest.deferred_tool_summaries)
        } else {
            None
        },
        ptc_tool_calling_guide: {
            #[cfg(feature = "ptc-runtime")]
            {
                manifest.ptc_tool_calling_guide.clone()
            }
            #[cfg(not(feature = "ptc-runtime"))]
            {
                None
            }
        },
    }
}

async fn load_skill_entries(
    workspace: Option<&WorkspaceBinding>,
    workspace_services: Option<&WorkspaceServices>,
    agent_type: Option<&str>,
    image_generation_available: bool,
) -> Vec<SkillSnapshotEntry> {
    let registry = get_skill_registry();
    let skills = match workspace {
        Some(workspace) if workspace.is_remote() => {
            if let Some(services) = workspace_services {
                registry
                    .get_implicitly_invocable_skills_for_remote_workspace(
                        services.fs.as_ref(),
                        &workspace.root_path_string(),
                        agent_type,
                    )
                    .await
            } else {
                Vec::new()
            }
        }
        Some(workspace) => {
            registry
                .get_implicitly_invocable_skills_for_workspace(Some(workspace), agent_type)
                .await
        }
        None => {
            registry
                .get_implicitly_invocable_skills_for_workspace(None, agent_type)
                .await
        }
    };

    openbitfun_agent_runtime::skills::filter_implicit_image_skills(
        skills,
        image_generation_available,
    )
    .into_iter()
    .map(skill_snapshot_entry_from_skill_info)
    .collect()
}

fn skill_snapshot_entry_from_skill_info(skill: SkillInfo) -> SkillSnapshotEntry {
    SkillSnapshotEntry {
        name: skill.name,
        description: skill.description,
        location: skill.path,
    }
}

async fn load_subagent_entries(
    workspace: Option<&WorkspaceBinding>,
    tool_context: &ToolUseContext,
    runtime_tool_restrictions: &ToolRuntimeRestrictions,
    external_agents: Option<&ExternalAgentCatalog>,
) -> Vec<AgentSnapshotEntry> {
    let registry = get_agent_registry();
    let workspace_id = workspace
        .filter(|workspace| !workspace.is_remote())
        .and_then(|workspace| workspace.workspace_id.as_deref());
    let agents = registry
        .get_subagents_for_query(&SubagentQueryContext {
            parent_agent_type: tool_context.agent_type.as_deref(),
            workspace_id,
            list_scope: SubagentListScope::TaskVisible,
            include_disabled: false,
            external_sources_supported: false,
        })
        .await;
    // Same gate as the AgentSpawn catalog, so the listing never offers an
    // external agent that admission would refuse.
    let agents =
        AgentExecutionTool::append_external_agents_from(external_agents, tool_context, agents)
            .await;

    agents
        .into_iter()
        .map(|agent| {
            let default_tools = agent
                .default_tools
                .into_iter()
                .filter(|tool_name| runtime_tool_restrictions.is_tool_allowed(tool_name))
                .collect();
            AgentSnapshotEntry {
                id: agent.id,
                description: agent.description,
                default_tools,
            }
        })
        .collect()
}

pub async fn build_embedded_user_context_reminder(
    workspace: Option<&WorkspaceBinding>,
    workspace_id: Option<&str>,
    session_id: &str,
    user_context_policy: &UserContextPolicy,
) -> Option<String> {
    let workspace = workspace?;
    let context = crate::agentic::agents::build_prompt_context_for_workspace(
        workspace,
        workspace_id,
        session_id,
        None,
        None,
        ToolListingSections::default(),
        Default::default(),
        user_context_policy,
    )
    .await?;
    PromptBuilder::new(context)
        .build_user_context_reminder(user_context_policy)
        .await
}

#[cfg(test)]
mod tests {
    use super::{build_tool_listing_sections, load_subagent_entries};
    use crate::agentic::skill_agent_snapshot::TurnSkillAgentSnapshot;
    use crate::agentic::tools::framework::ToolUseContext;
    use crate::agentic::tools::manifest_resolver::ResolvedToolManifest;
    use crate::agentic::tools::ToolRuntimeRestrictions;
    use crate::util::types::ToolDefinition;
    use openbitfun_agent_tools::GetToolSpecDeferredToolSummary;
    use serde_json::json;

    #[test]
    fn direct_tool_listing_uses_prompt_visible_manifest_definitions() {
        let manifest = ResolvedToolManifest {
            allowed_tool_names: vec!["AllowedButNotVisible".to_string()],
            tool_definitions: ["Read", "GetToolSpec", "CallDeferredTool"]
                .into_iter()
                .map(|name| ToolDefinition {
                    name: name.to_string(),
                    description: String::new(),
                    parameters: json!({ "type": "object" }),
                })
                .collect(),
            deferred_tool_names: vec!["WebFetch".to_string()],
            deferred_tool_summaries: vec![GetToolSpecDeferredToolSummary {
                name: "WebFetch".to_string(),
                short_description: Some("Fetch a web page".to_string()),
            }],
            #[cfg(feature = "ptc-runtime")]
            ptc_tool_calling_guide: None,
            catalog_generation: 7,
        };

        let sections = build_tool_listing_sections(&manifest, &TurnSkillAgentSnapshot::default());
        let direct_tools = sections
            .direct_tool_listing
            .expect("deferred manifests should include the direct tool listing");

        assert_eq!(
            direct_tools,
            "<direct_tools>\n- Read\n- GetToolSpec\n- CallDeferredTool\n</direct_tools>"
        );
        assert!(!direct_tools.contains("AllowedButNotVisible"));
    }

    #[tokio::test]
    async fn subagent_projection_hides_runtime_denied_tools() {
        let mut restrictions = ToolRuntimeRestrictions::default();
        restrictions
            .denied_tool_names
            .insert("ControlHub".to_string());

        let context = description_context("Claw", &restrictions);
        let agents = load_subagent_entries(None, &context, &restrictions, None).await;
        let computer_use = agents
            .iter()
            .find(|agent| agent.id == "ComputerUse")
            .expect("Claw should advertise the ComputerUse subagent");

        assert!(!computer_use
            .default_tools
            .iter()
            .any(|tool_name| !restrictions.is_tool_allowed(tool_name)));
    }

    fn description_context(
        agent_type: &str,
        restrictions: &ToolRuntimeRestrictions,
    ) -> ToolUseContext {
        crate::agentic::tools::tool_context_runtime::build_tool_description_context(
            agent_type,
            None,
            None,
            None,
            None,
            None,
            None,
            &std::collections::HashMap::new(),
            restrictions,
        )
    }

    struct ListedExternalAgent;

    #[async_trait::async_trait]
    impl openbitfun_runtime_ports::ExternalAgentExecutor for ListedExternalAgent {
        async fn availability(
            &self,
            _client_id: &str,
            _context: &openbitfun_runtime_ports::ExternalAgentContext,
        ) -> openbitfun_runtime_ports::ExternalAgentAvailability {
            openbitfun_runtime_ports::ExternalAgentAvailability::Available
        }

        async fn list_agents(
            &self,
            _listing: openbitfun_runtime_ports::ExternalAgentListing,
            _remote_connection_id: Option<&str>,
        ) -> Vec<openbitfun_runtime_ports::ExternalAgentDescriptor> {
            vec![openbitfun_runtime_ports::ExternalAgentDescriptor {
                client_id: "claude-code".to_string(),
                name: "Claude Code".to_string(),
                description: None,
                best_for: None,
                readonly: false,
                availability: openbitfun_runtime_ports::ExternalAgentAvailability::Available,
            }]
        }

        async fn run_turn(
            &self,
            _request: openbitfun_runtime_ports::ExternalAgentTurnRequest,
            _sink: &dyn openbitfun_runtime_ports::ExternalAgentEventSink,
        ) -> openbitfun_runtime_ports::PortResult<openbitfun_runtime_ports::ExternalAgentTurnOutcome>
        {
            unreachable!("listing never runs a turn")
        }

        async fn cancel(
            &self,
            _client_id: &str,
            _openbitfun_session_id: &str,
        ) -> openbitfun_runtime_ports::PortResult<bool> {
            Ok(false)
        }

        async fn close(
            &self,
            _client_id: &str,
            _openbitfun_session_id: &str,
        ) -> openbitfun_runtime_ports::PortResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn subagent_projection_lists_admissible_external_agents() {
        let restrictions = ToolRuntimeRestrictions::default();
        let catalog = crate::agentic::tools::implementations::task::ExternalAgentCatalog {
            executor: Some(std::sync::Arc::new(ListedExternalAgent)),
            remote_agents: None,
            parent_ceiling: Default::default(),
        };
        let executor = &catalog;

        let standard = description_context("Standard", &restrictions);
        let agents = load_subagent_entries(None, &standard, &restrictions, Some(executor)).await;
        assert!(
            agents.iter().any(|agent| agent.id == "acp:claude-code"),
            "the turn listing must offer the external agents AgentSpawn admits"
        );

        let without_executor = load_subagent_entries(None, &standard, &restrictions, None).await;
        assert!(!without_executor
            .iter()
            .any(|agent| agent.id.starts_with("acp:")));

        let review = description_context("DeepReview", &restrictions);
        let agents = load_subagent_entries(None, &review, &restrictions, Some(executor)).await;
        assert!(!agents.iter().any(|agent| agent.id.starts_with("acp:")));
    }

    struct ListedRemoteAgent;

    #[async_trait::async_trait]
    impl openbitfun_runtime_ports::RemoteAgentDelegationPort for ListedRemoteAgent {
        async fn list_agents(
            &self,
            _listing: openbitfun_runtime_ports::RemoteAgentListing,
        ) -> Vec<openbitfun_runtime_ports::RemoteAgentDescriptor> {
            vec![openbitfun_runtime_ports::RemoteAgentDescriptor {
                address: openbitfun_runtime_ports::RemoteAgentAddress {
                    device_id: "laptop".to_string(),
                    open_agent_id: "helper".to_string(),
                },
                device_name: "Laptop".to_string(),
                agent_type: "Standard".to_string(),
                display_name: "Helper".to_string(),
                description: None,
                task_kinds: vec![openbitfun_runtime_ports::RemoteAgentTaskKind::Message],
                availability: openbitfun_runtime_ports::RemoteAgentAvailability::Available,
            }]
        }

        async fn submit(
            &self,
            _request: openbitfun_runtime_ports::RemoteAgentSubmitRequest,
        ) -> openbitfun_runtime_ports::PortResult<openbitfun_runtime_ports::RemoteAgentJob>
        {
            unreachable!("listing never submits")
        }

        async fn observe(
            &self,
            _job: &openbitfun_runtime_ports::RemoteAgentJob,
            _cursor: u64,
        ) -> openbitfun_runtime_ports::PortResult<openbitfun_runtime_ports::RemoteAgentJobSnapshot>
        {
            unreachable!("listing never observes")
        }

        async fn cancel(
            &self,
            _job: &openbitfun_runtime_ports::RemoteAgentJob,
        ) -> openbitfun_runtime_ports::PortResult<bool> {
            Ok(false)
        }

        async fn answer(
            &self,
            _job: &openbitfun_runtime_ports::RemoteAgentJob,
            _request_id: &str,
            _reply: openbitfun_runtime_ports::PermissionReply,
        ) -> openbitfun_runtime_ports::PortResult<()> {
            Ok(())
        }

        async fn continue_job(
            &self,
            _job: &openbitfun_runtime_ports::RemoteAgentJob,
            _turn_id: &str,
            _prompt: String,
        ) -> openbitfun_runtime_ports::PortResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn subagent_projection_lists_admissible_remote_agents() {
        let restrictions = ToolRuntimeRestrictions::default();
        let catalog = crate::agentic::tools::implementations::task::ExternalAgentCatalog {
            executor: None,
            remote_agents: Some(std::sync::Arc::new(ListedRemoteAgent)),
            parent_ceiling: Default::default(),
        };
        let is_remote = |agent: &super::AgentSnapshotEntry| agent.id.starts_with("remote:");

        let standard = description_context("Standard", &restrictions);
        let agents = load_subagent_entries(None, &standard, &restrictions, Some(&catalog)).await;
        assert!(
            agents
                .iter()
                .any(|agent| agent.id == "remote:laptop/helper"),
            "the turn listing must offer the remote agents AgentSpawn admits"
        );

        let review = description_context("DeepReview", &restrictions);
        let agents = load_subagent_entries(None, &review, &restrictions, Some(&catalog)).await;
        assert!(!agents.iter().any(is_remote));

        // A turn another device delegated here never delegates onward.
        let mut delegated = description_context("Standard", &restrictions);
        delegated.custom_data.insert(
            crate::service::delegation::DISPATCH_DELEGATION_CONTEXT_KEY.to_string(),
            serde_json::Value::Bool(true),
        );
        let agents = load_subagent_entries(None, &delegated, &restrictions, Some(&catalog)).await;
        assert!(!agents.iter().any(is_remote));
    }
}
