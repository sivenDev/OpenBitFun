//! `openbitfun delegation`: inspect the assistants of this device and the
//! tasks other devices delegated to them. Sharing an assistant is done in the
//! desktop app; headless hosts do not share or discover assistants.

use anyhow::{anyhow, Result};
use clap::Subcommand;
use openbitfun_core::service::delegation::{assistant_directory, local_assistants};
use openbitfun_core::service::delegation_audit::{DelegationAuditEvent, DelegationAuditLog};

use crate::plugin_diagnostics::escape_terminal_text;

#[derive(Subcommand)]
pub(crate) enum DelegationAction {
    /// List the assistants on this device
    Assistants {
        #[arg(long)]
        json: bool,
    },
    /// Show the tasks other devices delegated to this device
    Audit {
        /// Print the entries as JSON, including unreadable lines
        #[arg(long)]
        json: bool,
        /// Show only the most recent entries
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

pub(crate) async fn run(action: DelegationAction) -> Result<()> {
    let path_manager = openbitfun_core::infrastructure::PathManager::new()
        .map_err(|error| anyhow!("resolve OpenBitFun storage root: {error}"))?;
    match action {
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
        DelegationAction::Audit { json, limit } => {
            let records = DelegationAuditLog::for_product_home(&path_manager).read()?;
            let shown = &records[records.len().saturating_sub(limit)..];
            if json {
                println!("{}", serde_json::to_string_pretty(shown)?);
                return Ok(());
            }
            if shown.is_empty() {
                println!("No tasks were delegated to this device by other devices.");
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
    }
    Ok(())
}
