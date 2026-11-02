//! Dispatch calls that arrive over a cross-account connection.
//!
//! The receiving host binds every such call to the connection it
//! authenticated (`__sourceConnectionId`). Before a verb runs, this module
//! limits what the connection may do: it acts only on jobs it started, it
//! cannot read files or inspect paths, and it cannot change the approval
//! policy or the model a job runs with. Approvals never come from a
//! connection; the host refuses `answer` before it reaches the runner.
//! [`present_response`] keeps the details of what this device's owner is
//! asked to approve out of every answer.

use anyhow::{bail, Context, Result};
use serde_json::Value;

use openbitfun_services_core::dispatch_contract::DISPATCH_SOURCE_CONNECTION_ARG;

use openbitfun_core::service::delegation::OpenAgentStore;

use super::store::{DispatchJobRecord, DispatchStore};

/// The connection a call is bound to, if any.
pub(super) fn source_connection(input: &Value) -> Option<String> {
    input
        .get(DISPATCH_SOURCE_CONNECTION_ARG)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|connection_id| !connection_id.is_empty())
        .map(str::to_string)
}

/// Whether `job` was submitted over `connection_id`.
pub(super) fn owns(job: &DispatchJobRecord, connection_id: &str) -> bool {
    job.request.source_connection_id.as_deref() == Some(connection_id)
}

fn job_id(input: &Value) -> &str {
    input
        .get("jobId")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// Another requester's job looks the same as a missing one.
fn missing(job_id: &str) -> anyhow::Error {
    anyhow::anyhow!("dispatch job '{job_id}' does not exist")
}

/// Refuses a call the connection may not make, and returns the input the
/// verb runs with. Verbs not listed here never run for a connection.
pub(super) async fn admit_call(
    store: &DispatchStore,
    grants: &OpenAgentStore,
    verb: &str,
    input: Value,
    connection_id: &str,
) -> Result<Value> {
    match verb {
        // Catalog and submit check the connection's grants themselves, and
        // submit refuses a workspace path; list shows only the connection's
        // jobs.
        "catalog" | "list" | "submit" => {}
        "probe" => {
            if input.get("workspacePath").is_some() {
                bail!("a connection cannot inspect paths on this device");
            }
        }
        "status" | "cancel" | "append" | "continue" | "query" => {
            let job_id = job_id(&input);
            let job = store
                .load_job(job_id)
                .ok()
                .filter(|job| owns(job, connection_id))
                .ok_or_else(|| missing(job_id))?;
            // New work on a job needs the grant it was admitted under.
            if matches!(verb, "append" | "continue") {
                ensure_still_granted(grants, &job, connection_id).await?;
            }
            match verb {
                "query" => {
                    if input.get("kind").and_then(Value::as_str) != Some("usageReport") {
                        bail!("a cross-account connection cannot read files on this device");
                    }
                }
                "continue" => {
                    for field in ["model", "reasoningPreset"] {
                        if !input.get(field).unwrap_or(&Value::Null).is_null() {
                            bail!("this device chooses the model of cross-account jobs");
                        }
                    }
                    if let Some(policy) = input.get("approvalPolicy").filter(|v| !v.is_null()) {
                        if policy.as_str() != Some("remote") {
                            bail!("cross-account jobs are approved only by this device's owner");
                        }
                    }
                }
                _ => {}
            }
        }
        other => bail!("dispatch verb '{other}' is not accepted over a cross-account connection"),
    }
    Ok(input)
}

/// What a connection receives back: approval prompts keep only what says the
/// job waits.
pub(super) fn present_response(verb: &str, _request: &Value, mut response: Value) -> Value {
    if verb == "status" {
        if let Some(pending) = response
            .get_mut("pendingPermissions")
            .and_then(Value::as_array_mut)
        {
            for request in pending.iter_mut() {
                *request = redacted_permission(request);
            }
        }
    }
    response
}

/// A pending prompt as another account sees it: that the job waits and on
/// what kind of action, never the paths, commands, or files involved.
fn redacted_permission(request: &Value) -> Value {
    let field = |name: &str| request.get(name).cloned().unwrap_or(Value::Null);
    serde_json::json!({
        "requestId": field("requestId"),
        "roundId": field("roundId"),
        "order": field("order"),
        "projectId": "",
        "sessionId": field("sessionId"),
        "agentId": field("agentId"),
        "action": field("action"),
        "resources": [],
        "saveResources": [],
        "source": field("source"),
    })
}

async fn ensure_still_granted(
    grants: &OpenAgentStore,
    job: &DispatchJobRecord,
    connection_id: &str,
) -> Result<()> {
    let delegation = job
        .request
        .delegation
        .as_ref()
        .context("a cross-account job has no delegation")?;
    grants
        .authorize_connection(connection_id, &delegation.open_agent_id)
        .await
        .context("read agents opened to other devices")?
        .map(|_| ())
        .map_err(anyhow::Error::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::protocol::{
        DispatchApprovalPolicy, DispatchDelegation, DispatchDelegationSource,
        DispatchSubmitRequest, DISPATCH_PROTOCOL_VERSION,
    };
    use serde_json::json;

    fn job(job_id: &str, connection_id: Option<&str>) -> DispatchSubmitRequest {
        DispatchSubmitRequest {
            protocol_version: DISPATCH_PROTOCOL_VERSION,
            job_id: job_id.to_string(),
            session_id: format!("session-{job_id}"),
            workspace_path: "/tmp/workspace".to_string(),
            agent_type: "assistant:workspace".to_string(),
            prompt: "hello".to_string(),
            approval_policy: DispatchApprovalPolicy::Remote,
            model: None,
            reasoning_preset: None,
            title: None,
            attachments: Vec::new(),
            setup_audit: Vec::new(),
            delegation: Some(DispatchDelegation {
                open_agent_id: "helper".to_string(),
                source: DispatchDelegationSource::default(),
            }),
            source_device_id: None,
            source_connection_id: connection_id.map(str::to_string),
            assistant_workspace: None,
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: DispatchStore,
        grants: OpenAgentStore,
    }

    /// Jobs of two connections and a local one; the assistant `helper` is
    /// granted to both connections, `notes` only to conn-3.
    async fn store_with_jobs() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
        let grants = OpenAgentStore::new(dir.path().join("open-agents.json"));
        let assistants = dir.path().join("personal_assistant");
        for (id, directory) in [("helper", "workspace"), ("notes", "workspace-notes")] {
            crate::dispatch::delegation::test_support::open_assistant(
                &grants,
                &assistants,
                id,
                directory,
                &[],
            )
            .await;
        }
        for connection_id in ["conn-1", "conn-2"] {
            grants
                .grant_connection("helper", connection_id)
                .await
                .unwrap();
        }
        grants.grant_connection("notes", "conn-3").await.unwrap();
        for (job_id, connection) in [
            ("delegation-mine", Some("conn-1")),
            ("delegation-theirs", Some("conn-2")),
            ("delegation-local", None),
        ] {
            let request = job(job_id, connection);
            store
                .create_job_with_intent(request.clone(), request, "title".to_string())
                .unwrap();
        }
        Fixture {
            _dir: dir,
            store,
            grants,
        }
    }

    #[tokio::test]
    async fn a_connection_acts_only_on_its_own_jobs() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        for verb in ["status", "cancel", "append", "continue"] {
            admit_call(
                store,
                &fixture.grants,
                verb,
                json!({ "jobId": "delegation-mine" }),
                "conn-1",
            )
            .await
            .unwrap();
            for other in ["delegation-theirs", "delegation-local", "missing"] {
                let error = admit_call(
                    store,
                    &fixture.grants,
                    verb,
                    json!({ "jobId": other }),
                    "conn-1",
                )
                .await
                .unwrap_err()
                .to_string();
                assert_eq!(error, format!("dispatch job '{other}' does not exist"));
            }
        }
        admit_call(store, &fixture.grants, "status", json!({}), "conn-1")
            .await
            .unwrap_err();
    }

    #[tokio::test]
    async fn new_work_on_a_job_needs_the_grant_it_was_admitted_under() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        let mine = json!({ "jobId": "delegation-mine" });
        fixture
            .grants
            .revoke_connection("helper", "conn-1")
            .await
            .unwrap();
        for verb in ["append", "continue"] {
            assert!(
                admit_call(store, &fixture.grants, verb, mine.clone(), "conn-1")
                    .await
                    .is_err()
            );
        }
        // The requester can still read and stop what is already running.
        for verb in ["status", "cancel"] {
            admit_call(store, &fixture.grants, verb, mine.clone(), "conn-1")
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn a_connection_cannot_read_files_or_change_approval_and_model() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        let mine = |extra: Value| {
            let mut input = json!({ "jobId": "delegation-mine" });
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            input
        };
        admit_call(
            store,
            &fixture.grants,
            "query",
            mine(json!({ "kind": "usageReport" })),
            "conn-1",
        )
        .await
        .unwrap();
        for kind in ["readFile", "readFileChunk"] {
            assert!(admit_call(
                store,
                &fixture.grants,
                "query",
                mine(json!({ "kind": kind })),
                "conn-1"
            )
            .await
            .is_err());
        }
        admit_call(
            store,
            &fixture.grants,
            "continue",
            mine(json!({ "approvalPolicy": "remote", "model": null })),
            "conn-1",
        )
        .await
        .unwrap();
        for extra in [
            json!({ "approvalPolicy": "auto" }),
            json!({ "model": "big-model" }),
            json!({ "reasoningPreset": "high" }),
        ] {
            assert!(
                admit_call(store, &fixture.grants, "continue", mine(extra), "conn-1")
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn pending_approvals_reach_a_connection_without_their_details() {
        let status = json!({
            "state": "running",
            "pendingPermissions": [{
                "requestId": "r1", "roundId": "round", "order": 0,
                "toolCallId": "tool", "projectPath": "/Users/owner/project",
                "projectId": "project", "sessionId": "s", "agentId": "agentic",
                "action": "bash", "resources": ["cat ~/.ssh/id_rsa"],
                "saveResources": ["cat *"], "source": { "kind": "tool_call", "identity": "bash" },
                "displayMetadata": { "command": "cat ~/.ssh/id_rsa" },
            }],
        });
        let shown = present_response("status", &json!({}), status);
        let text = shown.to_string();
        assert!(!text.contains("id_rsa"), "{text}");
        assert!(!text.contains("/Users/owner"), "{text}");
        let request: openbitfun_product_domains::tool_permissions::PermissionRequest =
            serde_json::from_value(shown["pendingPermissions"][0].clone()).unwrap();
        assert_eq!(request.request_id, "r1");
        assert_eq!(request.action, "bash");
    }

    #[tokio::test]
    async fn verbs_outside_the_connection_set_are_refused() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        for verb in ["probe", "catalog", "submit", "list"] {
            admit_call(store, &fixture.grants, verb, json!({}), "conn-1")
                .await
                .unwrap();
        }
        for verb in [
            "answer",
            "workspace-provision",
            "workspace-bundle-begin",
            "workspace-sync",
            "unknown",
        ] {
            assert!(admit_call(
                store,
                &fixture.grants,
                verb,
                json!({ "jobId": "delegation-mine" }),
                "conn-1"
            )
            .await
            .is_err());
        }
        assert_eq!(
            source_connection(&json!({ "__sourceConnectionId": " conn-1 " })).as_deref(),
            Some("conn-1")
        );
        assert_eq!(
            source_connection(&json!({ "__sourceConnectionId": "" })),
            None
        );
        assert_eq!(source_connection(&json!({})), None);
        // A connection never names a path on this device.
        for path in ["/etc", "connection-workspace:delegation-mine"] {
            assert!(admit_call(
                store,
                &fixture.grants,
                "probe",
                json!({ "workspacePath": path }),
                "conn-1"
            )
            .await
            .is_err());
        }
    }
}
