//! Dispatch calls that arrive over a cross-account connection.
//!
//! The receiving host binds every such call to the connection it
//! authenticated (`__sourceConnectionId`). Before a verb runs, this module
//! limits what the connection may do: it acts only on jobs it submitted, it
//! cannot read files, and it cannot change the approval policy or the model a
//! job runs with. Approvals never come from a connection; the host refuses
//! `answer` before it reaches the runner.

use anyhow::{bail, Result};
use serde_json::Value;

use openbitfun_services_core::dispatch_contract::DISPATCH_SOURCE_CONNECTION_ARG;

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

/// Refuses a call the connection may not make. Verbs not listed here never
/// run for a connection.
pub(super) fn admit_call(
    store: &DispatchStore,
    verb: &str,
    input: &Value,
    connection_id: &str,
) -> Result<()> {
    match verb {
        // Catalog and submit check the connection's grants themselves; list
        // shows only the connection's jobs.
        "probe" | "catalog" | "submit" | "list" => Ok(()),
        "status" | "cancel" | "append" | "continue" | "query" => {
            let job_id = input
                .get("jobId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Another requester's job looks the same as a missing one.
            let owned = store
                .load_job(job_id)
                .is_ok_and(|job| owns(&job, connection_id));
            if !owned {
                bail!("dispatch job '{job_id}' does not exist");
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
            Ok(())
        }
        other => bail!("dispatch verb '{other}' is not accepted over a cross-account connection"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::protocol::{
        DispatchApprovalPolicy, DispatchDelegation, DispatchDelegationSource,
        DispatchDelegationTaskKind, DispatchSubmitRequest, DISPATCH_PROTOCOL_VERSION,
    };
    use serde_json::json;

    fn job(job_id: &str, connection_id: Option<&str>) -> DispatchSubmitRequest {
        DispatchSubmitRequest {
            protocol_version: DISPATCH_PROTOCOL_VERSION,
            job_id: job_id.to_string(),
            session_id: format!("session-{job_id}"),
            workspace_path: "/tmp/workspace".to_string(),
            agent_type: "agentic".to_string(),
            prompt: "hello".to_string(),
            approval_policy: DispatchApprovalPolicy::Remote,
            model: None,
            reasoning_preset: None,
            title: None,
            attachments: Vec::new(),
            setup_audit: Vec::new(),
            delegation: Some(DispatchDelegation {
                open_agent_id: "helper".to_string(),
                task_kind: DispatchDelegationTaskKind::Message,
                source: DispatchDelegationSource::default(),
            }),
            source_device_id: None,
            source_connection_id: connection_id.map(str::to_string),
        }
    }

    fn store_with_jobs() -> (tempfile::TempDir, DispatchStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
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
        (dir, store)
    }

    #[test]
    fn a_connection_acts_only_on_its_own_jobs() {
        let (_dir, store) = store_with_jobs();
        for verb in ["status", "cancel", "append", "continue"] {
            admit_call(
                &store,
                verb,
                &json!({ "jobId": "delegation-mine" }),
                "conn-1",
            )
            .unwrap();
            for other in ["delegation-theirs", "delegation-local", "missing"] {
                let error = admit_call(&store, verb, &json!({ "jobId": other }), "conn-1")
                    .unwrap_err()
                    .to_string();
                assert_eq!(error, format!("dispatch job '{other}' does not exist"));
            }
        }
        admit_call(&store, "status", &json!({}), "conn-1").unwrap_err();
    }

    #[test]
    fn a_connection_cannot_read_files_or_change_approval_and_model() {
        let (_dir, store) = store_with_jobs();
        let mine = |extra: Value| {
            let mut input = json!({ "jobId": "delegation-mine" });
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            input
        };
        admit_call(
            &store,
            "query",
            &mine(json!({ "kind": "usageReport" })),
            "conn-1",
        )
        .unwrap();
        for kind in ["readFile", "readFileChunk"] {
            assert!(admit_call(&store, "query", &mine(json!({ "kind": kind })), "conn-1").is_err());
        }
        admit_call(
            &store,
            "continue",
            &mine(json!({ "approvalPolicy": "remote", "model": null })),
            "conn-1",
        )
        .unwrap();
        for extra in [
            json!({ "approvalPolicy": "auto" }),
            json!({ "model": "big-model" }),
            json!({ "reasoningPreset": "high" }),
        ] {
            assert!(admit_call(&store, "continue", &mine(extra), "conn-1").is_err());
        }
    }

    #[test]
    fn verbs_outside_the_connection_set_are_refused() {
        let (_dir, store) = store_with_jobs();
        for verb in ["probe", "catalog", "submit", "list"] {
            admit_call(&store, verb, &json!({}), "conn-1").unwrap();
        }
        for verb in ["answer", "workspace-provision", "workspace-sync", "unknown"] {
            assert!(admit_call(
                &store,
                verb,
                &json!({ "jobId": "delegation-mine" }),
                "conn-1"
            )
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
    }
}
