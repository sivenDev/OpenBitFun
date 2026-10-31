//! `openbitfun delegation`: manage the assistants this device opens to other
//! devices and accounts. Headless hosts (CLI Peer Host, the account daemon)
//! have no settings page, so this is their management entry.

use anyhow::{anyhow, bail, Result};
use clap::Subcommand;
use openbitfun_core::service::delegation::{
    assistant_agent_type, assistant_directory, local_assistants, ConnectionLimits,
    DelegationTaskKind, OpenAgent, OpenAgentRecord, OpenAgentStore, ALL_SAME_ACCOUNT_DEVICES,
};
use openbitfun_core::service::delegation_audit::{DelegationAuditEvent, DelegationAuditLog};

use crate::plugin_diagnostics::escape_terminal_text;

#[derive(Subcommand)]
pub(crate) enum DelegationAction {
    /// List opened assistants, including entries this build no longer accepts
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
        #[arg(long)]
        assistant: String,
        /// Name shown to other devices; defaults to the assistant's directory
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Same-account device id allowed to delegate (repeatable)
        #[arg(long = "device", required_unless_present = "all_devices")]
        devices: Vec<String>,
        /// Allow every device of this account
        #[arg(long, conflicts_with = "devices")]
        all_devices: bool,
        /// Confirm that delegated tasks may call external agents with this
        /// device's own login or credentials
        #[arg(long)]
        accept_usage: bool,
    },
    /// Close an opened assistant, or remove an entry no longer supported
    Close { id: String },
    /// Grant an opened assistant to a paired device of another account
    /// (see `openbitfun connections list`)
    GrantConnection {
        /// The opened assistant
        id: String,
        /// The connection to grant it to
        connection_id: String,
        /// Most tasks this connection may run at once with the assistant
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
    /// Withdraw an opened assistant from a connection
    RevokeConnection {
        /// The opened assistant
        id: String,
        /// The connection to withdraw it from
        connection_id: String,
    },
}

pub(crate) async fn run(action: DelegationAction) -> Result<()> {
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
            assistant: directory,
            name,
            description,
            devices,
            all_devices,
            accept_usage,
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
        DelegationAction::GrantConnection {
            id,
            connection_id,
            max_jobs,
            max_minutes,
            no_limits,
        } => {
            let connection = openbitfun_core::service::remote_connect::connection_rpc::grantable_connection(
                &openbitfun_core::service::remote_connect::connections::ConnectionStore::open_default()?,
                &connection_id,
            )?;
            let mut agent = store.grant_connection(&id, &connection_id).await?;
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
                bail!(
                    "no opened assistant with id '{}'",
                    escape_terminal_text(&id)
                );
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

fn print_records(records: &[OpenAgentRecord]) {
    if records.is_empty() {
        println!("No assistants are open to other devices.");
        return;
    }
    for record in records {
        match (&record.agent, &record.problem) {
            (Some(agent), _) => {
                println!(
                    "- {}: {} (assistant: {}, devices: {})",
                    escape_terminal_text(&agent.id),
                    escape_terminal_text(&agent.display_name),
                    escape_terminal_text(
                        assistant_directory(&agent.agent_type).unwrap_or_default()
                    ),
                    describe_devices(&agent.same_account_devices)
                );
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
