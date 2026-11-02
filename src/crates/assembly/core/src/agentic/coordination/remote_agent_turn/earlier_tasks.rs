//! Tasks this host delegated earlier, which a later conversation can resume.
//!
//! A job belongs to the conversation that last continued it: the newest
//! background task carrying its link. Resuming hands the job to the resuming
//! conversation, and the earlier one can no longer continue it.

use std::collections::HashMap;

use super::RemoteJobLink;
use crate::agentic::coordination::coordination_store::CoordinationStore;
use crate::util::errors::OpenBitFunResult;

/// Context key carrying the earlier task a new remote subagent resumes.
pub(crate) const RESUME_REMOTE_TASK_CONTEXT_KEY: &str = "remote_agent_resume_task_id";
/// Earlier tasks listed in a remote agent's catalog entry.
const LISTED_EARLIER_TASKS: usize = 5;
const SUMMARY_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EarlierRemoteTask {
    /// The link of the turn that last continued the job.
    pub(crate) link: RemoteJobLink,
    pub(crate) owner_child_session_id: Option<String>,
    /// The last turn has not ended on this host.
    pub(crate) running: bool,
    pub(crate) started_at_ms: i64,
    pub(crate) last_turn_at_ms: i64,
}

impl EarlierRemoteTask {
    /// Whether the conversation of `child_session_id` may continue the job.
    pub(crate) fn owned_by(&self, child_session_id: &str) -> bool {
        self.owner_child_session_id.as_deref() == Some(child_session_id)
    }
}

/// Every job this host delegated, newest first.
pub(crate) async fn earlier_remote_tasks(
    store: &CoordinationStore,
) -> OpenBitFunResult<Vec<EarlierRemoteTask>> {
    let mut tasks: Vec<EarlierRemoteTask> = Vec::new();
    let mut by_job: HashMap<String, usize> = HashMap::new();
    for row in store.outbound_remote_tasks().await? {
        let Some(link) = RemoteJobLink::parse(&row.remote_job) else {
            continue;
        };
        match by_job.get(&link.job.job_id) {
            Some(&index) => {
                let task = &mut tasks[index];
                task.started_at_ms = task.started_at_ms.min(row.created_at_ms);
            }
            None => {
                by_job.insert(link.job.job_id.clone(), tasks.len());
                tasks.push(EarlierRemoteTask {
                    link,
                    owner_child_session_id: row.child_session_id,
                    running: row.status == "running",
                    started_at_ms: row.created_at_ms,
                    last_turn_at_ms: row.created_at_ms,
                });
            }
        }
    }
    Ok(tasks)
}

pub(crate) async fn earlier_remote_task(
    store: &CoordinationStore,
    job_id: &str,
) -> OpenBitFunResult<Option<EarlierRemoteTask>> {
    Ok(earlier_remote_tasks(store)
        .await?
        .into_iter()
        .find(|task| task.link.job.job_id == job_id))
}

/// The start of a task's first prompt, on one line.
pub(crate) fn summarize_prompt(prompt: &str) -> String {
    let line = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= SUMMARY_CHARS {
        return line;
    }
    let mut summary: String = line.chars().take(SUMMARY_CHARS).collect();
    summary.push_str("...");
    summary
}

/// The catalog note listing the most recent tasks delegated to `agent_type`.
pub(crate) fn describe_earlier_tasks(
    tasks: &[EarlierRemoteTask],
    agent_type: &str,
) -> Option<String> {
    let lines = tasks
        .iter()
        .filter(|task| task.link.agent_type == agent_type)
        .take(LISTED_EARLIER_TASKS)
        .map(|task| {
            let running = if task.running { ", still running" } else { "" };
            format!(
                "- `{}` (started {}, last turn {}{running}): {}",
                task.link.job.job_id,
                day(task.started_at_ms),
                day(task.last_turn_at_ms),
                task.link.summary
            )
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| {
        format!(
            "Earlier tasks delegated to this agent, newest first. To continue one, spawn a new agent of this type with `resume_task_id`; it picks up the earlier session on that device, unless the device has deleted it since:\n{}",
            lines.join("\n")
        )
    })
}

fn day(at_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(at_ms)
        .map(|at| at.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
