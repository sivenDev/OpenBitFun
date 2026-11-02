//! Dispatch calls from a device that uses one of this device's shared
//! assistants.
//!
//! The LAN share host binds every such call to the sharing key the channel
//! verified (`__sourceShareKey`). Before a verb runs, this module limits what
//! that device may do: it acts only on jobs it started, it cannot read files
//! or inspect paths, and it cannot change the approval policy or the model a
//! job runs with. Approvals never come from such a device; the host refuses
//! `answer` before it reaches the runner. [`present_response`] keeps the
//! details of what this device's owner is asked to approve out of every
//! answer.

use anyhow::{bail, Context, Result};
use serde_json::Value;

use openbitfun_services_core::dispatch_contract::DISPATCH_SOURCE_SHARE_KEY_ARG;

use openbitfun_core::service::lan_share::SharedAssistantStore;

use super::store::{DispatchJobRecord, DispatchStore};

/// The sharing key a call is bound to, if any, in lowercase hex.
pub(super) fn source_share_key(input: &Value) -> Option<String> {
    input
        .get(DISPATCH_SOURCE_SHARE_KEY_ARG)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_ascii_lowercase)
}

/// Whether `job` was submitted by the device with `share_key`.
pub(super) fn owns(job: &DispatchJobRecord, share_key: &str) -> bool {
    job.request
        .source_share_key
        .as_deref()
        .is_some_and(|key| key.eq_ignore_ascii_case(share_key))
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

/// Refuses a call the device may not make, and returns the input the verb
/// runs with. Verbs not listed here never run for such a device.
pub(super) async fn admit_call(
    store: &DispatchStore,
    grants: &SharedAssistantStore,
    verb: &str,
    input: Value,
    share_key: &str,
) -> Result<Value> {
    match verb {
        // Catalog and submit check the device's grants themselves, and
        // submit refuses a workspace path; list shows only the device's jobs.
        "catalog" | "list" | "submit" => {}
        "probe" => {
            if input.get("workspacePath").is_some() {
                bail!("a device using a shared assistant cannot inspect paths here");
            }
        }
        "status" | "cancel" | "append" | "continue" | "query" => {
            let job_id = job_id(&input);
            let job = store
                .load_job(job_id)
                .ok()
                .filter(|job| owns(job, share_key))
                .ok_or_else(|| missing(job_id))?;
            // New work on a job needs the grant it was admitted under.
            if matches!(verb, "append" | "continue") {
                ensure_still_granted(grants, &job, share_key).await?;
            }
            match verb {
                "query" => {
                    if input.get("kind").and_then(Value::as_str) != Some("usageReport") {
                        bail!("a device using a shared assistant cannot read files here");
                    }
                }
                "continue" => {
                    for field in ["model", "reasoningPreset"] {
                        if !input.get(field).unwrap_or(&Value::Null).is_null() {
                            bail!("this device chooses the model of shared assistants' tasks");
                        }
                    }
                    if let Some(policy) = input.get("approvalPolicy").filter(|v| !v.is_null()) {
                        if policy.as_str() != Some("remote") {
                            bail!(
                                "shared assistants' tasks are approved only by this device's owner"
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        other => {
            bail!("dispatch verb '{other}' is not accepted from a device using a shared assistant")
        }
    }
    Ok(input)
}

/// What such a device receives back: approval prompts keep only what says
/// the job waits.
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

/// A pending prompt as another device sees it: that the job waits and on
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
    grants: &SharedAssistantStore,
    job: &DispatchJobRecord,
    share_key: &str,
) -> Result<()> {
    let delegation = job
        .request
        .delegation
        .as_ref()
        .context("a task from another device has no delegation")?;
    grants
        .authorize(&delegation.assistant_id, share_key)
        .await
        .context("read shared assistants")?
        .map(|_| ())
        .map_err(anyhow::Error::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::delegation::test_support::share_key as key;
    use crate::dispatch::protocol::{
        DispatchApprovalPolicy, DispatchDelegation, DispatchDelegationSource,
        DispatchSubmitRequest, DISPATCH_PROTOCOL_VERSION,
    };
    use serde_json::json;

    fn job(job_id: &str, share_key: Option<String>) -> DispatchSubmitRequest {
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
                assistant_id: "workspace".to_string(),
                source: DispatchDelegationSource::default(),
            }),
            source_share_key: share_key,
            source_user: None,
            assistant_workspace: None,
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: DispatchStore,
        grants: SharedAssistantStore,
    }

    /// Jobs of devices 1 and 2 and a local one; the assistant `workspace` is
    /// shared with devices 1 and 2, `workspace-notes` only with device 3.
    async fn store_with_jobs() -> Fixture {
        use crate::dispatch::delegation::test_support::share_assistant;
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
        let grants = SharedAssistantStore::new(dir.path().join("shared-assistants.json"));
        let assistants = dir.path().join("personal_assistant");
        for (directory, seeds) in [
            ("workspace", &[1u8, 2][..]),
            ("workspace-notes", &[3u8][..]),
        ] {
            share_assistant(&grants, &assistants, directory, Default::default(), seeds).await;
        }
        for (job_id, seed) in [
            ("delegation-mine", Some(1)),
            ("delegation-theirs", Some(2)),
            ("delegation-local", None),
        ] {
            let request = job(job_id, seed.map(key));
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
    async fn a_device_acts_only_on_its_own_jobs() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        for verb in ["status", "cancel", "append", "continue"] {
            admit_call(
                store,
                &fixture.grants,
                verb,
                json!({ "jobId": "delegation-mine" }),
                &key(1),
            )
            .await
            .unwrap();
            for other in ["delegation-theirs", "delegation-local", "missing"] {
                let error = admit_call(
                    store,
                    &fixture.grants,
                    verb,
                    json!({ "jobId": other }),
                    &key(1),
                )
                .await
                .unwrap_err()
                .to_string();
                assert_eq!(error, format!("dispatch job '{other}' does not exist"));
            }
        }
        admit_call(store, &fixture.grants, "status", json!({}), &key(1))
            .await
            .unwrap_err();
    }

    #[tokio::test]
    async fn new_work_on_a_job_needs_the_grant_it_was_admitted_under() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        let mine = json!({ "jobId": "delegation-mine" });
        fixture.grants.revoke("workspace", &key(1)).await.unwrap();
        for verb in ["append", "continue"] {
            assert!(
                admit_call(store, &fixture.grants, verb, mine.clone(), &key(1))
                    .await
                    .is_err()
            );
        }
        // The requester can still read and stop what is already running.
        for verb in ["status", "cancel"] {
            admit_call(store, &fixture.grants, verb, mine.clone(), &key(1))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn a_device_cannot_read_files_or_change_approval_and_model() {
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
            &key(1),
        )
        .await
        .unwrap();
        for kind in ["readFile", "readFileChunk"] {
            assert!(admit_call(
                store,
                &fixture.grants,
                "query",
                mine(json!({ "kind": kind })),
                &key(1)
            )
            .await
            .is_err());
        }
        admit_call(
            store,
            &fixture.grants,
            "continue",
            mine(json!({ "approvalPolicy": "remote", "model": null })),
            &key(1),
        )
        .await
        .unwrap();
        for extra in [
            json!({ "approvalPolicy": "auto" }),
            json!({ "model": "big-model" }),
            json!({ "reasoningPreset": "high" }),
        ] {
            assert!(
                admit_call(store, &fixture.grants, "continue", mine(extra), &key(1))
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn pending_approvals_reach_a_device_without_their_details() {
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
    async fn verbs_outside_the_share_set_are_refused() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        for verb in ["probe", "catalog", "submit", "list"] {
            admit_call(store, &fixture.grants, verb, json!({}), &key(1))
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
                &key(1)
            )
            .await
            .is_err());
        }
        let upper = key(1).to_uppercase();
        assert_eq!(
            source_share_key(&json!({ "__sourceShareKey": format!(" {upper} ") })),
            Some(key(1))
        );
        assert_eq!(source_share_key(&json!({ "__sourceShareKey": "" })), None);
        assert_eq!(source_share_key(&json!({})), None);
        // Such a device never names a path on this device.
        for path in ["/etc", "/Users"] {
            assert!(admit_call(
                store,
                &fixture.grants,
                "probe",
                json!({ "workspacePath": path }),
                &key(1)
            )
            .await
            .is_err());
        }
    }
}
