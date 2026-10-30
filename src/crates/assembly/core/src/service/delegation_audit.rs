//! Target-side record of tasks other accounts delegated to this device.
//!
//! Each cross-account task appends one entry when it is admitted and one when
//! it ends. The log is append-only JSON lines under the product home and is
//! read tolerantly: an unreadable line is kept and reported, never dropped,
//! so the owner can see that something was there.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::infrastructure::PathManager;

/// What an audit entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationAuditEvent {
    /// The task was admitted and its job created.
    Submitted,
    /// The task's turn ended; `state` says how.
    Finished,
    /// An event from a newer build.
    #[serde(other)]
    Unknown,
}

/// One cross-account task event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationAuditEntry {
    /// RFC 3339 time of the event.
    pub at: String,
    pub event: DelegationAuditEvent,
    pub job_id: String,
    pub connection_id: String,
    #[serde(default)]
    pub peer_user_id: String,
    #[serde(default)]
    pub peer_display_name: String,
    #[serde(default)]
    pub peer_device_id: String,
    #[serde(default)]
    pub peer_device_name: String,
    pub open_agent_id: String,
    pub task_kind: String,
    /// The job state a `finished` entry ended in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Why the task ended, when it did not succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One line of the log as read back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationAuditRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<DelegationAuditEntry>,
    /// Set when the line could not be read; the line is kept as `raw`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// The device-local delegation audit log.
#[derive(Debug, Clone)]
pub struct DelegationAuditLog {
    path: PathBuf,
}

impl DelegationAuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_product_home(path_manager: &PathManager) -> Self {
        Self::new(
            path_manager
                .product_home_dir()
                .join("delegation")
                .join("audit.jsonl"),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends `entry` as one line. The file is private to this user.
    pub fn append(&self, entry: &DelegationAuditEntry) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
        line.push('\n');
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&self.path)?.write_all(line.as_bytes())
    }

    /// Every line, oldest first; a missing log reads as empty.
    pub fn read(&self) -> std::io::Result<Vec<DelegationAuditRecord>> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        Ok(text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(
                |line| match serde_json::from_str::<DelegationAuditEntry>(line) {
                    Ok(entry) => DelegationAuditRecord {
                        entry: Some(entry),
                        problem: None,
                        raw: None,
                    },
                    Err(error) => DelegationAuditRecord {
                        entry: None,
                        problem: Some(format!("unreadable audit entry: {error}")),
                        raw: Some(line.to_string()),
                    },
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(event: DelegationAuditEvent, state: Option<&str>) -> DelegationAuditEntry {
        DelegationAuditEntry {
            at: "2026-10-30T10:00:00Z".to_string(),
            event,
            job_id: "delegation-1".to_string(),
            connection_id: "conn-1".to_string(),
            peer_user_id: "2002".to_string(),
            peer_display_name: "Bea".to_string(),
            peer_device_id: "device-b".to_string(),
            peer_device_name: "B1 Desktop".to_string(),
            open_agent_id: "helper".to_string(),
            task_kind: "message".to_string(),
            state: state.map(str::to_string),
            detail: None,
        }
    }

    #[test]
    fn entries_are_appended_and_unreadable_lines_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let log = DelegationAuditLog::new(dir.path().join("delegation").join("audit.jsonl"));
        assert!(log.read().unwrap().is_empty());
        log.append(&entry(DelegationAuditEvent::Submitted, None))
            .unwrap();
        // A damaged line and an event from a newer build.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(log.path())
            .unwrap();
        writeln!(file, "{{not json").unwrap();
        let mut newer = serde_json::to_value(entry(DelegationAuditEvent::Submitted, None)).unwrap();
        newer["event"] = "paused".into();
        newer["futureField"] = true.into();
        writeln!(file, "{newer}").unwrap();
        log.append(&entry(DelegationAuditEvent::Finished, Some("succeeded")))
            .unwrap();

        let records = log.read().unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(
            records[0].entry.as_ref().unwrap().event,
            DelegationAuditEvent::Submitted
        );
        assert!(records[1].problem.is_some());
        assert_eq!(records[1].raw.as_deref(), Some("{not json"));
        assert_eq!(
            records[2].entry.as_ref().unwrap().event,
            DelegationAuditEvent::Unknown
        );
        assert_eq!(
            records[3].entry.as_ref().unwrap().state.as_deref(),
            Some("succeeded")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(log.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
