//! The owner's view of tasks other devices gave this device's shared
//! assistants.
//!
//! Tasks run in the same `openbitfun dispatch` runner that accepted them, so
//! these commands read and answer them through that runner as the device
//! owner. Nothing is written to the assistant's own session store.

use openbitfun_core::infrastructure::PathManager;
use openbitfun_core::service::delegation::{
    local_assistants, LocalAssistant, DELEGATED_JOB_ID_PREFIX,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The device a task came from, as its grant named it when the task was
/// admitted.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedTaskSource {
    pub device_id: String,
    pub fingerprint: String,
    pub display_name: String,
    pub device_name: String,
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
    /// The shared assistant: its directory.
    pub assistant_id: String,
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

/// The assistants on this device that can be shared.
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
    Ok(delegated_tasks(&listed))
}

/// The delegated tasks in a `dispatch list` answer, each followed by the
/// sessions its calls to this device's other assistants run in, listed under
/// the called assistant.
fn delegated_tasks(listed: &Value) -> Vec<DelegatedTask> {
    let text =
        |value: &Value, field: &str| value.get(field).and_then(Value::as_str).map(str::to_string);
    let mut tasks = Vec::new();
    for entry in listed.as_array().into_iter().flatten() {
        let Some(delegation) = entry.get("delegation") else {
            continue;
        };
        let Some(job_id) = text(entry, "jobId") else {
            continue;
        };
        let user = delegation.get("sourceUser").unwrap_or(&Value::Null);
        let source = || DelegatedTaskSource {
            device_id: text(user, "deviceId").unwrap_or_default(),
            fingerprint: text(user, "fingerprint").unwrap_or_default(),
            display_name: text(user, "displayName").unwrap_or_default(),
            device_name: text(user, "deviceName").unwrap_or_default(),
        };
        let state = text(entry, "state").unwrap_or_default();
        let started_at = text(entry, "startedAt");
        let pending_permission_count = entry
            .get("pendingPermissionCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        tasks.push(DelegatedTask {
            job_id: job_id.clone(),
            session_id: text(entry, "sessionId").unwrap_or_default(),
            state: state.clone(),
            started_at: started_at.clone(),
            title: text(entry, "title").unwrap_or_default(),
            assistant_id: text(delegation, "assistantId").unwrap_or_default(),
            agent_type: text(entry, "agentType").unwrap_or_default(),
            assistant_workspace: text(delegation, "assistantWorkspace"),
            pending_permission_count,
            source: source(),
        });
        for call in delegation
            .get("calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(assistant_id), Some(session_id)) =
                (text(call, "assistantId"), text(call, "sessionId"))
            else {
                continue;
            };
            if assistant_id.is_empty() || session_id.is_empty() {
                continue;
            }
            tasks.push(DelegatedTask {
                job_id: job_id.clone(),
                session_id,
                state: state.clone(),
                started_at: started_at.clone(),
                title: text(call, "title").unwrap_or_default(),
                agent_type: format!("assistant:{assistant_id}"),
                assistant_id,
                assistant_workspace: None,
                // The task's actions wait on the task's own row; counted
                // once there.
                pending_permission_count: 0,
                source: source(),
            });
        }
    }
    tasks
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
    fn delegated_tasks_name_their_assistant_and_device() {
        let listed = serde_json::json!([
            {
                "jobId": "delegation-1",
                "sessionId": "s1",
                "state": "running",
                "title": "Plan",
                "agentType": "assistant:workspace",
                "pendingPermissionCount": 2,
                "delegation": {
                    "assistantId": "workspace",
                    "assistantWorkspace": "/home/me/personal_assistant/workspace",
                    "sourceShareKey": "ab",
                    "sourceUser": {
                        "deviceId": "0123",
                        "fingerprint": "AB23CD",
                        "displayName": "Bea",
                        "deviceName": "Bea's Mac"
                    },
                    "source": {}
                }
            },
            { "jobId": "dispatch-2", "state": "running" }
        ]);
        let tasks = delegated_tasks(&listed);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].assistant_id, "workspace");
        assert_eq!(tasks[0].pending_permission_count, 2);
        assert_eq!(tasks[0].source.display_name, "Bea");
        assert_eq!(tasks[0].source.device_name, "Bea's Mac");
        assert_eq!(tasks[0].source.fingerprint, "AB23CD");
    }

    #[test]
    fn a_tasks_calls_are_listed_under_the_called_assistant() {
        let listed = serde_json::json!([{
            "jobId": "delegation-1",
            "sessionId": "s1",
            "state": "completed",
            "title": "Plan",
            "agentType": "assistant:workspace",
            "delegation": {
                "assistantId": "workspace",
                "sourceUser": { "deviceId": "0123", "displayName": "Bea" },
                "source": {},
                "calls": [
                    {
                        "assistantId": "workspace-notes",
                        "workspacePath": "/home/me/delegation/tenants/workspace-notes/0123",
                        "sessionId": "called",
                        "title": "Write a line"
                    },
                    { "workspacePath": "/elsewhere", "sessionId": "shared-call" }
                ]
            }
        }]);
        let tasks = delegated_tasks(&listed);
        assert_eq!(tasks.len(), 2);
        let call = &tasks[1];
        assert_eq!(call.job_id, "delegation-1");
        assert_eq!(call.session_id, "called");
        assert_eq!(call.agent_type, "assistant:workspace-notes");
        assert_eq!(call.assistant_id, "workspace-notes");
        assert_eq!(call.title, "Write a line");
        assert_eq!(call.state, "completed");
        assert_eq!(call.pending_permission_count, 0);
        assert_eq!(call.source.display_name, "Bea");
    }

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
