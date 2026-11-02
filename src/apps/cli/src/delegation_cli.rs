//! `openbitfun delegation`: manage the assistants this device opens to other
//! devices and accounts. Headless hosts (CLI Peer Host, the account daemon)
//! have no settings page, so this is their management entry.

use anyhow::{anyhow, bail, Result};
use clap::Subcommand;
use openbitfun_core::service::delegation::{
    assistant_agent_type, assistant_directory, local_assistants, ConnectionLimits,
    ConnectionTrustStore, DelegationTrust, OpenAgent, OpenAgentStore, ALL_SAME_ACCOUNT_DEVICES,
};
use openbitfun_core::service::delegation_audit::{DelegationAuditEvent, DelegationAuditLog};
use std::collections::BTreeMap;

use crate::plugin_diagnostics::escape_terminal_text;

#[derive(Subcommand)]
pub(crate) enum DelegationAction {
    /// List opened assistants
    List {
        /// Print the opened assistants as JSON
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
    /// Close an opened assistant
    Close { id: String },
    /// Grant an opened assistant to a paired device of another account
    /// (see `openbitfun connections list`)
    GrantConnection {
        /// The opened assistant
        id: String,
        /// The connection to grant it to
        connection_id: String,
        /// Most tasks this connection may run at once with the assistant
        /// (1-100); more wait, queued, until one finishes. Setting either
        /// limit replaces both; granting again without limits keeps the
        /// current ones.
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
    /// Choose how far tasks from a paired device of another account are
    /// trusted. Normal (the default) keeps them away from your desktop,
    /// other sessions, schedules, publishing, profile and memory; full lets
    /// them run like your own
    TrustConnection {
        /// The connection (see `openbitfun connections list`)
        connection_id: String,
        #[arg(long, value_enum)]
        level: CliTrustLevel,
        /// Confirm the risk of full trust
        #[arg(long)]
        accept_risk: bool,
    },
    /// Withdraw an opened assistant from a connection
    RevokeConnection {
        /// The opened assistant
        id: String,
        /// The connection to withdraw it from
        connection_id: String,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum CliTrustLevel {
    Normal,
    Full,
}

const FULL_TRUST_NOTICE: &str = "Full trust lets tasks from this connection run like your own. \
They can read your profile (USER.md) and memory, which every assistant on this device shares, \
and use your other sessions, this desktop, schedules and publishing. \
Trust an account fully only if you trust it as much as yourself.";

const NORMAL_TRUST_NOTICE: &str =
    "At normal trust, tasks cannot use your desktop, other sessions, \
schedules or publishing, get no profile or memory in their prompt, and need your approval to read \
them. Commands and external agents they start are protected only by your approval.";

pub(crate) async fn run(action: DelegationAction) -> Result<()> {
    let path_manager = openbitfun_core::infrastructure::PathManager::new()
        .map_err(|error| anyhow!("resolve OpenBitFun storage root: {error}"))?;
    let store = OpenAgentStore::for_product_home(&path_manager);
    let trust = ConnectionTrustStore::for_product_home(&path_manager);
    match action {
        DelegationAction::List { json } => {
            let agents = store.agents().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&agents)?);
            } else {
                print_agents(&agents, &trust.levels().await.unwrap_or_default());
            }
        }
        DelegationAction::TrustConnection {
            connection_id,
            level,
            accept_risk,
        } => {
            let known = openbitfun_core::service::remote_connect::connections::ConnectionStore::open_default()
                .and_then(|store| store.load())
                .map_err(|error| anyhow!("read this device's connections: {error}"))?
                .find(&connection_id)
                .is_some();
            if !known {
                bail!(
                    "connection {} does not exist on this device",
                    escape_terminal_text(&connection_id)
                );
            }
            let level = match level {
                CliTrustLevel::Normal => DelegationTrust::Normal,
                CliTrustLevel::Full => {
                    if !accept_risk {
                        println!("{FULL_TRUST_NOTICE}");
                        bail!("run again with --accept-risk to trust this connection fully");
                    }
                    DelegationTrust::Full
                }
            };
            trust.set(&connection_id, level).await?;
            println!(
                "Connection {} is trusted {}; this applies from each task's next turn.",
                escape_terminal_text(&connection_id),
                level.as_str()
            );
            if level == DelegationTrust::Normal {
                println!("{NORMAL_TRUST_NOTICE}");
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
                };
                println!(
                    "- {} {} task {} for {} ({}), device {}, via {} with {}",
                    escape_terminal_text(&entry.at),
                    event,
                    escape_terminal_text(&entry.job_id),
                    escape_terminal_text(&entry.peer_display_name),
                    escape_terminal_text(&entry.peer_user_id),
                    escape_terminal_text(&entry.peer_device_name),
                    escape_terminal_text(&entry.connection_id),
                    escape_terminal_text(&entry.open_agent_id),
                );
                println!("  trust: {}", escape_terminal_text(&entry.trust));
                if !entry.external_agents.is_empty() {
                    println!(
                        "  external agents: {}",
                        escape_terminal_text(&entry.external_agents.join(", "))
                    );
                }
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

fn print_agents(agents: &[OpenAgent], trust: &BTreeMap<String, DelegationTrust>) {
    if agents.is_empty() {
        println!("No assistants are open to other devices.");
        return;
    }
    for agent in agents {
        println!(
            "- {}: {} (assistant: {}, devices: {})",
            escape_terminal_text(&agent.id),
            escape_terminal_text(&agent.display_name),
            escape_terminal_text(assistant_directory(&agent.agent_type).unwrap_or_default()),
            describe_devices(&agent.same_account_devices)
        );
        for grant in &agent.connections {
            println!(
                "  granted to connection: {} ({}, trust: {})",
                escape_terminal_text(&grant.connection_id),
                describe_limits(grant.limits),
                trust
                    .get(&grant.connection_id)
                    .copied()
                    .unwrap_or_default()
                    .as_str()
            );
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
