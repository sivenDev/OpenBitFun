//! The owner's view of tasks other devices and accounts delegated to the
//! assistants of this device.
//!
//! Tasks run in the same `openbitfun dispatch` runner that accepted them, so
//! these commands read and answer them through that runner as the device
//! owner. Nothing is written to the assistant's own session store.

use std::time::Duration;

use openbitfun_core::infrastructure::PathManager;
use openbitfun_core::service::delegation::{
    local_assistants, LocalAssistant, DELEGATED_JOB_ID_PREFIX,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long the owner's list waits for this account's device names.
const DEVICE_NAMES_TIMEOUT: Duration = Duration::from_secs(5);

/// Who delegated a task, as far as this device knows it.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedTaskSource {
    /// `device` for a device of this account, `connection` for a paired
    /// device of another account.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_user_id: Option<String>,
}

/// One task delegated to an assistant here.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedTask {
    pub job_id: String,
    pub session_id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    pub title: String,
    pub open_agent_id: String,
    /// The assistant the task runs as, `assistant:<directory>`.
    pub agent_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_workspace: Option<String>,
    pub pending_permission_count: usize,
    pub source: DelegatedTaskSource,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationTaskStatusRequest {
    pub job_id: String,
    #[serde(default)]
    pub cursor: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationTaskAnswerRequest {
    pub job_id: String,
    pub request_id: String,
    /// `once` or `reject`: the owner approves each action of another
    /// device's task one at a time.
    pub reply: String,
}

/// The assistants on this device that can be opened.
#[tauri::command]
pub async fn delegation_list_assistants() -> Result<Vec<LocalAssistant>, String> {
    let path_manager =
        PathManager::new().map_err(|error| format!("resolve storage root: {error}"))?;
    Ok(local_assistants(&path_manager))
}

/// Tasks delegated to this device's assistants, newest first.
#[tauri::command]
pub async fn delegation_list_tasks() -> Result<Vec<DelegatedTask>, String> {
    let listed = super::dispatch_host::run_local("list", serde_json::json!({}))
        .await
        .map_err(|error| format!("{error:#}"))?;
    let device_names = tokio::time::timeout(
        DEVICE_NAMES_TIMEOUT,
        super::remote_connect_api::account_list_devices(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default();
    Ok(listed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let delegation = entry.get("delegation")?;
            let text = |value: &Value, field: &str| {
                value.get(field).and_then(Value::as_str).map(str::to_string)
            };
            let mut source = DelegatedTaskSource::default();
            if let Some(connection_id) = text(delegation, "sourceConnectionId") {
                source.kind = "connection";
                source.connection_id = Some(connection_id);
            } else {
                source.kind = "device";
                source.device_id = text(delegation, "sourceDeviceId");
                source.device_name = source.device_id.as_ref().and_then(|device_id| {
                    device_names
                        .iter()
                        .find(|device| &device.device_id == device_id)
                        .map(|device| device.device_name.clone())
                });
            }
            Some(DelegatedTask {
                job_id: text(entry, "jobId")?,
                session_id: text(entry, "sessionId").unwrap_or_default(),
                state: text(entry, "state").unwrap_or_default(),
                started_at: text(entry, "startedAt"),
                title: text(entry, "title").unwrap_or_default(),
                open_agent_id: text(delegation, "openAgentId").unwrap_or_default(),
                agent_type: text(entry, "agentType").unwrap_or_default(),
                assistant_workspace: text(delegation, "assistantWorkspace"),
                pending_permission_count: entry
                    .get("pendingPermissionCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize,
                source,
            })
        })
        .collect())
}

fn ensure_delegated(job_id: &str) -> Result<(), String> {
    if job_id.starts_with(DELEGATED_JOB_ID_PREFIX) {
        Ok(())
    } else {
        Err(format!("'{job_id}' is not a delegated task"))
    }
}

/// A delegated task's events and pending prompts, in full: the owner sees
/// what the requester never does.
#[tauri::command]
pub async fn delegation_task_status(request: DelegationTaskStatusRequest) -> Result<Value, String> {
    ensure_delegated(&request.job_id)?;
    super::dispatch_host::run_local(
        "status",
        serde_json::json!({ "jobId": request.job_id, "cursor": request.cursor }),
    )
    .await
    .map_err(|error| format!("{error:#}"))
}

/// Answers a prompt of a delegated task as the owner of this device.
#[tauri::command]
pub async fn delegation_task_answer(request: DelegationTaskAnswerRequest) -> Result<Value, String> {
    ensure_delegated(&request.job_id)?;
    if !matches!(request.reply.as_str(), "once" | "reject") {
        return Err(format!(
            "'{}' is not an answer to a delegated task; use once or reject",
            request.reply
        ));
    }
    super::dispatch_host::run_local(
        "answer",
        serde_json::json!({
            "jobId": request.job_id,
            "requestId": request.request_id,
            "reply": request.reply,
        }),
    )
    .await
    .map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_delegated_tasks_are_read_or_answered_here() {
        assert!(ensure_delegated(&format!("{DELEGATED_JOB_ID_PREFIX}1")).is_ok());
        // The owner's own dispatch jobs are managed where they were sent from.
        assert!(ensure_delegated("dispatch-1").is_err());
    }

    #[tokio::test]
    async fn answers_approve_one_action_at_a_time() {
        let answer = |reply: &str| DelegationTaskAnswerRequest {
            job_id: format!("{DELEGATED_JOB_ID_PREFIX}1"),
            request_id: "request-1".to_string(),
            reply: reply.to_string(),
        };
        let error = delegation_task_answer(answer("always"))
            .await
            .expect_err("a remembered grant is refused");
        assert!(error.contains("once or reject"), "{error}");
    }
}
