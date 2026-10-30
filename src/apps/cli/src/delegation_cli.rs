//! `openbitfun delegation`: manage the agents this device opens to other
//! devices for delegated dispatch jobs. Headless hosts (CLI Peer Host, the
//! account daemon) have no settings page, so this is their management entry.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Subcommand, ValueEnum};
use openbitfun_core::service::delegation::{
    DelegationTaskKind, OpenAgent, OpenAgentRecord, OpenAgentStore, ALL_SAME_ACCOUNT_DEVICES,
};

use crate::plugin_diagnostics::escape_terminal_text;

#[derive(Subcommand)]
pub(crate) enum DelegationAction {
    /// List agents opened to other devices, including unreadable entries
    List {
        /// Print the stored entries as JSON
        #[arg(long)]
        json: bool,
    },
    /// Open a local agent mode to other devices of this account
    Open {
        /// Stable id other devices address (letters, digits, '-' or '_')
        id: String,
        /// Local agent mode that runs delegated jobs
        #[arg(long = "agent")]
        agent_type: String,
        /// Name shown to other devices; defaults to the agent mode's name
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Task kinds other devices may delegate (repeatable)
        #[arg(long = "task-kind", value_enum, required = true)]
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
    },
    /// Close an opened agent
    Close { id: String },
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
        DelegationAction::Open {
            id,
            agent_type,
            name,
            description,
            task_kinds,
            workspace,
            devices,
            all_devices,
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
            let agent = store
                .open_local_agent(OpenAgent {
                    id,
                    agent_type,
                    display_name: name.unwrap_or_default(),
                    description,
                    task_kinds: task_kinds.into_iter().map(Into::into).collect(),
                    message_workspace,
                    same_account_devices: if all_devices {
                        vec![ALL_SAME_ACCOUNT_DEVICES.to_string()]
                    } else {
                        devices
                    },
                })
                .await?;
            println!(
                "Opened {} ({}) to {}",
                escape_terminal_text(&agent.id),
                escape_terminal_text(&agent.agent_type),
                describe_devices(&agent.same_account_devices)
            );
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
                    .filter_map(|kind| match kind {
                        DelegationTaskKind::Message => Some("message"),
                        DelegationTaskKind::Coding => Some("coding"),
                        DelegationTaskKind::Unknown => None,
                    })
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
