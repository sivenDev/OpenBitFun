//! `openbitfun delegation`: manage the agents this device opens to other
//! devices for delegated dispatch jobs. Headless hosts (CLI Peer Host, the
//! account daemon) have no settings page, so this is their management entry.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Subcommand, ValueEnum};
use openbitfun_core::service::delegation::{
    assistant_agent_type, assistant_directory, local_assistants, ConnectionLimits,
    DelegationTaskKind, ExternalAgentUsageNotice, OpenAgent, OpenAgentRecord, OpenAgentStore,
    ALL_SAME_ACCOUNT_DEVICES,
};
use openbitfun_core::service::delegation_audit::{DelegationAuditEvent, DelegationAuditLog};

use crate::plugin_diagnostics::escape_terminal_text;

#[derive(Subcommand)]
pub(crate) enum DelegationAction {
    /// List agents opened to other devices, including unreadable entries
    List {
        /// Print the stored entries as JSON
        #[arg(long)]
        json: bool,
    },
    /// List the assistants on this device that can be opened
    Assistants {
        #[arg(long)]
        json: bool,
    },
    /// Open an assistant to other devices of this account
    Open {
        /// Stable id other devices address (letters, digits, '-' or '_')
        id: String,
        /// The assistant that answers delegated tasks: its directory as
        /// `openbitfun delegation assistants` lists it (`workspace` is the
        /// default assistant)
        #[arg(long, conflicts_with = "agent_type")]
        assistant: Option<String>,
        /// Local agent mode or external agent that runs delegated jobs
        #[arg(long = "agent", required_unless_present = "assistant")]
        agent_type: Option<String>,
        /// Name shown to other devices; defaults to the agent's name
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Task kinds other devices may delegate (repeatable); an assistant
        /// takes message tasks only
        #[arg(long = "task-kind", value_enum, required_unless_present = "assistant")]
        task_kinds: Vec<CliDelegationTaskKind>,
        /// Existing directory message tasks run in. Without one, each
        /// requesting device gets its own empty directory.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Same-account device id allowed to delegate (repeatable)
        #[arg(long = "device", required_unless_present = "all_devices")]
        devices: Vec<String>,
        /// Allow every device of this account
        #[arg(long, conflicts_with = "devices")]
        all_devices: bool,
        /// Confirm the usage notice of an external agent: delegated jobs run
        /// it with this device's own login or credentials
        #[arg(long)]
        accept_usage: bool,
    },
    /// Close an opened agent
    Close { id: String },
    /// Grant an opened agent to a paired device of another account
    /// (see `openbitfun connections list`)
    GrantConnection {
        /// The opened agent
        id: String,
        /// The connection to grant it to
        connection_id: String,
        /// Confirm the usage notice of an external agent again for this
        /// connection: its jobs run on this device's own login or credentials
        #[arg(long)]
        accept_usage: bool,
        /// Most tasks this connection may run at once with the agent
        /// (1-100). Setting either limit replaces both; granting again
        /// without limits keeps the current ones.
        #[arg(long)]
        max_jobs: Option<u32>,
        /// Minutes one turn may run before it is stopped (1-1440)
        #[arg(long)]
        max_minutes: Option<u32>,
        /// Remove both limits
        #[arg(long, conflicts_with_all = ["max_jobs", "max_minutes"])]
        no_limits: bool,
    },
    /// Show the tasks other accounts delegated to this device
    Audit {
        /// Print the entries as JSON, including unreadable lines
        #[arg(long)]
        json: bool,
        /// Show only the most recent entries
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Withdraw an opened agent from a connection
    RevokeConnection {
        /// The opened agent
        id: String,
        /// The connection to withdraw it from
        connection_id: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum CliDelegationTaskKind {
    Message,
    Coding,
}

impl From<CliDelegationTaskKind> for DelegationTaskKind {
    fn from(kind: CliDelegationTaskKind) -> Self {
        match kind {
            CliDelegationTaskKind::Message => Self::Message,
            CliDelegationTaskKind::Coding => Self::Coding,
        }
    }
}

pub(crate) async fn run(action: DelegationAction) -> Result<()> {
    // The agent registry resolves modes for the active delivery profile.
    crate::agent::agentic_system::select_agentic_system_profile(
        openbitfun_core::product_assembly::DeliveryProfile::Cli,
    )?;
    let path_manager = openbitfun_core::infrastructure::PathManager::new()
        .map_err(|error| anyhow!("resolve OpenBitFun storage root: {error}"))?;
    let store = OpenAgentStore::for_product_home(&path_manager);
    match action {
        DelegationAction::List { json } => {
            let records = store.records().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&records)?);
            } else {
                print_records(&records);
            }
        }
        DelegationAction::Assistants { json } => {
            let assistants = local_assistants(&path_manager);
            if json {
                println!("{}", serde_json::to_string_pretty(&assistants)?);
            } else if assistants.is_empty() {
                println!("No assistants on this device.");
            } else {
                for assistant in &assistants {
                    println!(
                        "- {} ({})",
                        escape_terminal_text(
                            assistant_directory(&assistant.agent_type).unwrap_or_default()
                        ),
                        escape_terminal_text(&assistant.root_path.to_string_lossy())
                    );
                }
            }
        }
        DelegationAction::Open {
            id,
            assistant: Some(directory),
            name,
            description,
            devices,
            all_devices,
            accept_usage,
            ..
        } => {
            if !accept_usage {
                println!("{ASSISTANT_USAGE_NOTICE}");
                bail!("run again with --accept-usage to open this assistant");
            }
            let entry = OpenAgent {
                id,
                agent_type: assistant_agent_type(directory.trim()),
                display_name: name.unwrap_or_default(),
                description,
                task_kinds: vec![DelegationTaskKind::Message],
                message_workspace: None,
                same_account_devices: if all_devices {
                    vec![ALL_SAME_ACCOUNT_DEVICES.to_string()]
                } else {
                    devices
                },
                connections: Vec::new(),
            };
            let agent = store
                .open_assistant(
                    &path_manager.assistant_workspace_base_dir(None),
                    entry,
                    true,
                )
                .await?;
            println!(
                "Opened assistant {} ({}) to {}",
                escape_terminal_text(&agent.id),
                escape_terminal_text(&agent.display_name),
                describe_devices(&agent.same_account_devices)
            );
        }
        DelegationAction::Open {
            id,
            assistant: None,
            agent_type,
            name,
            description,
            task_kinds,
            workspace,
            devices,
            all_devices,
            accept_usage,
        } => {
            let message_workspace = workspace
                .map(|path| {
                    let path = std::fs::canonicalize(&path)
                        .with_context(|| format!("resolve workspace {}", path.display()))?;
                    if !path.is_dir() {
                        bail!("workspace {} is not a directory", path.display());
                    }
                    Ok(path.to_string_lossy().into_owned())
                })
                .transpose()?;
            let entry = OpenAgent {
                id,
                agent_type: agent_type.unwrap_or_default(),
                display_name: name.unwrap_or_default(),
                description,
                task_kinds: task_kinds.into_iter().map(Into::into).collect(),
                message_workspace,
                same_account_devices: if all_devices {
                    vec![ALL_SAME_ACCOUNT_DEVICES.to_string()]
                } else {
                    devices
                },
                connections: Vec::new(),
            };
            let agent = match ExternalAgentUsageNotice::for_agent_type(&entry.agent_type) {
                Some(notice) if !accept_usage => {
                    println!("{}", usage_notice_text(notice));
                    bail!("run again with --accept-usage to open this external agent");
                }
                Some(_) => {
                    let executor = crate::agent::agentic_system::external_agent_executor().await;
                    store
                        .open_external_agent(entry, true, executor.as_deref())
                        .await?
                }
                None => store.open_local_agent(entry).await?,
            };
            println!(
                "Opened {} ({}) to {}",
                escape_terminal_text(&agent.id),
                escape_terminal_text(&agent.agent_type),
                describe_devices(&agent.same_account_devices)
            );
        }
        DelegationAction::GrantConnection {
            id,
            connection_id,
            accept_usage,
            max_jobs,
            max_minutes,
            no_limits,
        } => {
            let connection = openbitfun_core::service::remote_connect::connection_rpc::grantable_connection(
                &openbitfun_core::service::remote_connect::connections::ConnectionStore::open_default()?,
                &connection_id,
            )?;
            let agent_type = store
                .records()
                .await?
                .into_iter()
                .filter_map(|record| record.agent)
                .find(|agent| agent.id == id)
                .map(|agent| agent.agent_type);
            if let Some(notice) = agent_type
                .as_deref()
                .and_then(ExternalAgentUsageNotice::for_agent_type)
            {
                if !accept_usage {
                    println!(
                        "{} ({} of another account) would run it.",
                        usage_notice_text(notice),
                        escape_terminal_text(&connection.peer_display_name)
                    );
                    bail!("run again with --accept-usage to grant this external agent");
                }
            }
            let mut agent = store
                .grant_connection(&id, &connection_id, accept_usage)
                .await?;
            if no_limits || max_jobs.is_some() || max_minutes.is_some() {
                let limits = ConnectionLimits {
                    max_concurrent_jobs: max_jobs,
                    max_job_minutes: max_minutes,
                };
                agent = store
                    .set_connection_limits(&id, &connection_id, limits)
                    .await?;
            }
            println!(
                "Granted {} to {} ({}), device {}",
                escape_terminal_text(&id),
                escape_terminal_text(&connection.peer_display_name),
                escape_terminal_text(&connection.peer_user_id),
                escape_terminal_text(&connection.peer_device_name)
            );
            if let Some(grant) = agent.connection_grant(&connection_id) {
                println!("  limits: {}", describe_limits(grant.limits));
            }
        }
        DelegationAction::RevokeConnection { id, connection_id } => {
            if store.revoke_connection(&id, &connection_id).await? {
                println!(
                    "Withdrew {} from connection {}",
                    escape_terminal_text(&id),
                    escape_terminal_text(&connection_id)
                );
            } else {
                bail!(
                    "{} is not granted to connection {}",
                    escape_terminal_text(&id),
                    escape_terminal_text(&connection_id)
                );
            }
        }
        DelegationAction::Audit { json, limit } => {
            let records = DelegationAuditLog::for_product_home(&path_manager).read()?;
            let shown = &records[records.len().saturating_sub(limit)..];
            if json {
                println!("{}", serde_json::to_string_pretty(shown)?);
                return Ok(());
            }
            if shown.is_empty() {
                println!("No tasks were delegated to this device by other accounts.");
            }
            for record in shown {
                let Some(entry) = &record.entry else {
                    println!(
                        "- unreadable entry, kept: {}",
                        escape_terminal_text(record.problem.as_deref().unwrap_or("unknown"))
                    );
                    continue;
                };
                let event = match entry.event {
                    DelegationAuditEvent::Submitted => "submitted".to_string(),
                    DelegationAuditEvent::Finished => {
                        format!("finished {}", entry.state.as_deref().unwrap_or("?"))
                    }
                    DelegationAuditEvent::Unknown => "other event".to_string(),
                };
                println!(
                    "- {} {} {} task {} for {} ({}), device {}, via {} with {}",
                    escape_terminal_text(&entry.at),
                    event,
                    escape_terminal_text(&entry.task_kind),
                    escape_terminal_text(&entry.job_id),
                    escape_terminal_text(&entry.peer_display_name),
                    escape_terminal_text(&entry.peer_user_id),
                    escape_terminal_text(&entry.peer_device_name),
                    escape_terminal_text(&entry.connection_id),
                    escape_terminal_text(&entry.open_agent_id),
                );
                if let Some(detail) = &entry.detail {
                    println!("  {}", escape_terminal_text(detail));
                }
            }
        }
        DelegationAction::Close { id } => {
            if store.close(&id).await? {
                println!("Closed {}", escape_terminal_text(&id));
            } else {
                bail!("no opened agent with id '{}'", escape_terminal_text(&id));
            }
        }
    }
    Ok(())
}

fn describe_limits(limits: ConnectionLimits) -> String {
    let jobs = limits
        .max_concurrent_jobs
        .map_or("any number of tasks".to_string(), |jobs| {
            format!("at most {jobs} task(s) at once")
        });
    let minutes = limits
        .max_job_minutes
        .map_or("no time limit".to_string(), |minutes| {
            format!("{minutes} minute(s) per turn")
        });
    format!("{jobs}, {minutes}")
}

/// What the owner confirms before opening an assistant.
const ASSISTANT_USAGE_NOTICE: &str = "Delegated tasks run this assistant on this device. \
     The assistant may hand work to external agents such as Claude Code or Codex with this device's own login or credentials: \
     that uses your quota with their providers, so check that their terms of service allow this use. \
     A Claude subscription login is for your own everyday use; an API key is recommended for delegated tasks.";

fn usage_notice_text(notice: ExternalAgentUsageNotice) -> &'static str {
    match notice {
        ExternalAgentUsageNotice::ProviderQuota => {
            "Delegated jobs run this external agent with this device's own login or credentials. \
             They use your quota with the agent's provider; check that its terms of service allow this use."
        }
        ExternalAgentUsageNotice::ClaudeCodeSubscription => {
            "Delegated jobs run Claude Code with this device's own login or credentials. \
             They use your quota with Anthropic; check that its terms of service allow this use. \
             A Claude subscription login is for your own everyday use; an API key is recommended for delegated jobs."
        }
    }
}

fn print_records(records: &[OpenAgentRecord]) {
    if records.is_empty() {
        println!("No agents are open to other devices.");
        return;
    }
    for record in records {
        match (&record.agent, &record.problem) {
            (Some(agent), _) => {
                let kinds: Vec<&str> = agent
                    .task_kinds
                    .iter()
                    .filter_map(|kind| kind.wire_name())
                    .collect();
                println!(
                    "- {}: {} (agent: {}, tasks: {}, devices: {})",
                    escape_terminal_text(&agent.id),
                    escape_terminal_text(&agent.display_name),
                    escape_terminal_text(&agent.agent_type),
                    kinds.join(", "),
                    describe_devices(&agent.same_account_devices)
                );
                if let Some(workspace) = &agent.message_workspace {
                    println!("  message workspace: {}", escape_terminal_text(workspace));
                }
                for grant in &agent.connections {
                    println!(
                        "  granted to connection: {} ({})",
                        escape_terminal_text(&grant.connection_id),
                        describe_limits(grant.limits)
                    );
                }
            }
            (None, problem) => {
                let id = record.id().unwrap_or("<no id>");
                println!(
                    "- {}: closed, {}",
                    escape_terminal_text(id),
                    escape_terminal_text(problem.as_deref().unwrap_or("unreadable entry"))
                );
            }
        }
    }
}

fn describe_devices(devices: &[String]) -> String {
    if devices
        .iter()
        .any(|device| device == ALL_SAME_ACCOUNT_DEVICES)
    {
        "all devices of this account".to_string()
    } else if devices.is_empty() {
        "no devices".to_string()
    } else {
        devices
            .iter()
            .map(|device| escape_terminal_text(device))
            .collect::<Vec<_>>()
            .join(", ")
    }
}
