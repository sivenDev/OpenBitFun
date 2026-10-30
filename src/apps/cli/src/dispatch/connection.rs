//! Dispatch calls that arrive over a cross-account connection.
//!
//! The receiving host binds every such call to the connection it
//! authenticated (`__sourceConnectionId`). Before a verb runs, this module
//! limits what the connection may do: it acts only on jobs and coding
//! workspaces it started, it cannot read files, and it cannot change the
//! approval policy or the model a job runs with. Approvals never come from a
//! connection; the host refuses `answer` before it reaches the runner.
//!
//! Coding workspaces are provisioned before their job exists, so the
//! connection that provisions one is recorded in its job directory, and only
//! a connection granted coding tasks may provision. Its repository cache is
//! kept apart from every other requester's, and it is seeded only from the
//! bundle the requester uploads: the target never fetches a URL another
//! account names with its owner's Git credentials. The connection names the
//! workspace by an opaque handle; [`present_response`] keeps this device's
//! paths, and the details of what its owner is asked to approve, out of every
//! answer.

use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use openbitfun_services_core::dispatch_contract::{
    DISPATCH_CONNECTION_WORKSPACE_PREFIX, DISPATCH_SOURCE_CONNECTION_ARG,
};

use openbitfun_core::service::delegation::{DelegationTaskKind, OpenAgentStore};

use super::protocol::DispatchDelegationTaskKind;
use super::store::{create_private_dir, DispatchJobRecord, DispatchStore};

/// Names the connection that provisioned a coding workspace.
const WORKSPACE_OWNER_FILE: &str = "source-connection";

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
    mut input: Value,
    connection_id: &str,
) -> Result<Value> {
    match verb {
        // Catalog and submit check the connection's grants themselves; list
        // shows only the connection's jobs.
        "catalog" | "list" => {}
        "probe" => {
            if let Some(handle) = input.get("workspacePath").and_then(Value::as_str) {
                let path = resolve_workspace_handle(store, handle, connection_id)?;
                input["workspacePath"] = Value::String(path);
            }
        }
        "submit" => {
            let handle = input
                .get("workspacePath")
                .and_then(Value::as_str)
                .filter(|path| path.starts_with(DISPATCH_CONNECTION_WORKSPACE_PREFIX))
                .map(str::to_string);
            if let Some(handle) = handle {
                if handle_job_id(&handle) != Some(job_id(&input)) {
                    bail!("coding tasks run only in the worktree provisioned for this job");
                }
                let path = resolve_workspace_handle(store, &handle, connection_id)?;
                input["workspacePath"] = Value::String(path);
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
        "workspace-provision" => {
            ensure_coding_grant(grants, connection_id).await?;
            claim_workspace(store, job_id(&input), connection_id)?;
            let fields = input
                .as_object_mut()
                .context("dispatch workspace provision request is not an object")?;
            if let Some(repo_key) = fields.get("repoKey").and_then(Value::as_str) {
                let isolated = isolated_repo_key(connection_id, repo_key);
                fields.insert("repoKey".into(), Value::String(isolated));
            }
            fields.insert("remoteUrl".into(), Value::Null);
        }
        "workspace-bundle-begin"
        | "workspace-bundle-chunk"
        | "workspace-bundle-commit"
        | "workspace-sync"
        | "workspace-sync-chunk" => {
            let job_id = job_id(&input);
            if workspace_owner(store, job_id)?.as_deref() != Some(connection_id) {
                return Err(missing(job_id));
            }
            ensure_coding_grant(grants, connection_id).await?;
        }
        other => bail!("dispatch verb '{other}' is not accepted over a cross-account connection"),
    }
    Ok(input)
}

/// What a connection receives back: this device's paths become the
/// workspace handle, and approval prompts keep only what says the job waits.
pub(super) fn present_response(verb: &str, request: &Value, mut response: Value) -> Value {
    match verb {
        "workspace-provision" => {
            if response.get("workspacePath").is_some_and(Value::is_string) {
                response["workspacePath"] = Value::String(workspace_handle(job_id(request)));
            }
        }
        "probe" => {
            if let Some(workspace) = response.get_mut("workspace").and_then(Value::as_object_mut) {
                let handle = request
                    .get("workspacePath")
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                workspace.insert("path".into(), handle);
            }
        }
        "status" => {
            if let Some(pending) = response
                .get_mut("pendingPermissions")
                .and_then(Value::as_array_mut)
            {
                for request in pending.iter_mut() {
                    *request = redacted_permission(request);
                }
            }
        }
        _ => {}
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

fn workspace_handle(job_id: &str) -> String {
    format!("{DISPATCH_CONNECTION_WORKSPACE_PREFIX}{job_id}")
}

fn handle_job_id(handle: &str) -> Option<&str> {
    handle
        .strip_prefix(DISPATCH_CONNECTION_WORKSPACE_PREFIX)
        .filter(|job_id| !job_id.is_empty())
}

/// The provisioned worktree a connection's handle names, if that connection
/// provisioned it.
fn resolve_workspace_handle(
    store: &DispatchStore,
    handle: &str,
    connection_id: &str,
) -> Result<String> {
    let refused = || anyhow::anyhow!("a connection cannot inspect paths on this device");
    let job_id = handle_job_id(handle).ok_or_else(refused)?;
    if workspace_owner(store, job_id)?.as_deref() != Some(connection_id) {
        return Err(missing(job_id));
    }
    super::workspace::provisioned_workspace_path(store, job_id)?
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow::anyhow!("the workspace for '{job_id}' is not provisioned yet"))
}

/// Coding workspaces are only for connections granted coding tasks.
async fn ensure_coding_grant(grants: &OpenAgentStore, connection_id: &str) -> Result<()> {
    let granted = grants
        .granted_to_connection(connection_id)
        .await
        .context("read agents opened to other devices")?
        .iter()
        .any(|agent| agent.allows_task_kind(DelegationTaskKind::Coding));
    if granted {
        Ok(())
    } else {
        bail!("no agent on this device takes coding tasks from this connection")
    }
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
    let kind = match delegation.task_kind {
        DispatchDelegationTaskKind::Message => DelegationTaskKind::Message,
        DispatchDelegationTaskKind::Coding => DelegationTaskKind::Coding,
    };
    grants
        .authorize_connection(connection_id, &delegation.open_agent_id, kind)
        .await
        .context("read agents opened to other devices")?
        .map(|_| ())
        .map_err(anyhow::Error::new)
}

/// A coding job submitted over a connection runs only in the workspace that
/// connection provisioned for the same job.
pub(super) fn ensure_provisioned_workspace(
    store: &DispatchStore,
    job_id: &str,
    connection_id: &str,
    workspace_path: &str,
) -> Result<()> {
    let refused =
        || anyhow::anyhow!("coding tasks run only in the worktree provisioned for this job");
    if workspace_owner(store, job_id)?.as_deref() != Some(connection_id) {
        return Err(refused());
    }
    let provisioned = super::workspace::provisioned_workspace_path(store, job_id)?
        .and_then(|path| path.canonicalize().ok())
        .ok_or_else(refused)?;
    let requested = Path::new(workspace_path.trim())
        .canonicalize()
        .map_err(|_| refused())?;
    if requested != provisioned {
        return Err(refused());
    }
    Ok(())
}

fn workspace_owner(store: &DispatchStore, job_id: &str) -> Result<Option<String>> {
    let path = store
        .workspace_upload_dir(job_id)?
        .join(WORKSPACE_OWNER_FILE);
    match std::fs::read_to_string(&path) {
        Ok(owner) => Ok(Some(owner.trim().to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Records `connection_id` as the owner of a new job's workspace, or checks
/// that it already is. A workspace someone else started is refused.
fn claim_workspace(store: &DispatchStore, job_id: &str, connection_id: &str) -> Result<()> {
    if job_id.is_empty() {
        return Err(missing(job_id));
    }
    if store
        .load_job(job_id)
        .is_ok_and(|job| !owns(&job, connection_id))
    {
        return Err(missing(job_id));
    }
    let job_dir = store.workspace_upload_dir(job_id)?;
    let owner_path = job_dir.join(WORKSPACE_OWNER_FILE);
    if job_dir.exists() && !owner_path.exists() {
        return Err(missing(job_id));
    }
    create_private_dir(&job_dir)?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&owner_path)
    {
        Ok(mut file) => {
            file.write_all(connection_id.as_bytes())
                .with_context(|| format!("write {}", owner_path.display()))?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if workspace_owner(store, job_id)?.as_deref() == Some(connection_id) {
                Ok(())
            } else {
                Err(missing(job_id))
            }
        }
        Err(error) => Err(error).with_context(|| format!("create {}", owner_path.display())),
    }
}

/// The repository cache key for a connection: a digest of the requester's
/// key and the connection, so no two requesters share a clone.
fn isolated_repo_key(connection_id: &str, repo_key: &str) -> String {
    Sha256::digest(format!("connection\n{connection_id}\n{repo_key}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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

    struct Fixture {
        _dir: tempfile::TempDir,
        store: DispatchStore,
        grants: OpenAgentStore,
    }

    /// Jobs of two connections and a local one; `helper` takes message and
    /// coding tasks from both connections, `notes` only messages from conn-3.
    async fn store_with_jobs() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
        let grants = OpenAgentStore::new(dir.path().join("open-agents.json"));
        grants
            .open(openbitfun_core::service::delegation::OpenAgent {
                id: "helper".into(),
                agent_type: "agentic".into(),
                display_name: "Helper".into(),
                description: None,
                task_kinds: vec![DelegationTaskKind::Message, DelegationTaskKind::Coding],
                message_workspace: None,
                same_account_devices: Vec::new(),
                connections: Vec::new(),
            })
            .await
            .unwrap();
        for connection_id in ["conn-1", "conn-2"] {
            grants
                .grant_connection("helper", connection_id, false)
                .await
                .unwrap();
        }
        grants
            .open(openbitfun_core::service::delegation::OpenAgent {
                id: "notes".into(),
                agent_type: "agentic".into(),
                display_name: "Notes".into(),
                description: None,
                task_kinds: vec![DelegationTaskKind::Message],
                message_workspace: None,
                same_account_devices: Vec::new(),
                connections: Vec::new(),
            })
            .await
            .unwrap();
        grants
            .grant_connection("notes", "conn-3", false)
            .await
            .unwrap();
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

    async fn provision(fixture: &Fixture, job_id: &str, connection_id: &str) -> Result<Value> {
        let request = json!({
            "jobId": job_id,
            "repoKey": "abcdef0123456789",
            "remoteUrl": "git@example.com:owner/private.git",
        });
        admit_call(
            &fixture.store,
            &fixture.grants,
            "workspace-provision",
            request,
            connection_id,
        )
        .await
    }

    #[tokio::test]
    async fn a_connection_uses_only_coding_workspaces_it_provisioned() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        let admitted = provision(&fixture, "delegation-new", "conn-1")
            .await
            .unwrap();
        // The target never fetches a URL another account names.
        assert_eq!(admitted["remoteUrl"], Value::Null);
        // Each connection gets its own repository cache.
        let key = admitted["repoKey"].as_str().unwrap().to_string();
        assert_eq!(key.len(), 64);
        assert_ne!(key, "abcdef0123456789");
        assert_eq!(
            provision(&fixture, "delegation-new", "conn-1")
                .await
                .unwrap()["repoKey"],
            key
        );
        assert_ne!(
            provision(&fixture, "delegation-other", "conn-2")
                .await
                .unwrap()["repoKey"],
            key
        );

        // Retries by the owner pass; anyone else sees no such job.
        for verb in [
            "workspace-bundle-begin",
            "workspace-bundle-chunk",
            "workspace-bundle-commit",
            "workspace-sync",
            "workspace-sync-chunk",
        ] {
            admit_call(
                store,
                &fixture.grants,
                verb,
                json!({ "jobId": "delegation-new" }),
                "conn-1",
            )
            .await
            .unwrap();
            let error = admit_call(
                store,
                &fixture.grants,
                verb,
                json!({ "jobId": "delegation-new" }),
                "conn-2",
            )
            .await
            .unwrap_err()
            .to_string();
            assert_eq!(error, "dispatch job 'delegation-new' does not exist");
        }
        assert!(provision(&fixture, "delegation-new", "conn-2")
            .await
            .is_err());
        // Jobs and workspaces another requester started cannot be claimed.
        assert!(provision(&fixture, "delegation-theirs", "conn-1")
            .await
            .is_err());
        std::fs::create_dir_all(
            fixture
                .store
                .workspace_upload_dir("delegation-local-coding")
                .unwrap(),
        )
        .unwrap();
        assert!(provision(&fixture, "delegation-local-coding", "conn-1")
            .await
            .is_err());

        // A coding submit must name the worktree provisioned for its own job.
        let error = ensure_provisioned_workspace(store, "delegation-new", "conn-1", "/tmp")
            .unwrap_err()
            .to_string();
        assert!(error.contains("provisioned for this job"), "{error}");
        assert!(
            ensure_provisioned_workspace(store, "delegation-local-coding", "conn-1", "/tmp")
                .is_err()
        );
    }

    #[tokio::test]
    async fn workspaces_are_named_by_handle_and_only_for_coding_grants() {
        let fixture = store_with_jobs().await;
        let store = &fixture.store;
        // A connection granted only message tasks cannot provision.
        let error = provision(&fixture, "delegation-x", "conn-3")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("coding tasks"), "{error}");

        provision(&fixture, "delegation-new", "conn-1")
            .await
            .unwrap();
        let worktree = fixture
            ._dir
            .path()
            .join("worktrees")
            .join("repo")
            .join("job");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            store
                .workspace_upload_dir("delegation-new")
                .unwrap()
                .join("provision.json"),
            serde_json::to_vec(&json!({
                "protocolVersion": 1, "jobId": "delegation-new", "repoKey": "abcdef0123456789",
                "baseCommit": "0123456789abcdef0123456789abcdef01234567", "branch": "b",
                "createdAt": "now", "workspacePath": worktree,
            }))
            .unwrap(),
        )
        .unwrap();
        let path = worktree.to_string_lossy().into_owned();

        // The provision answer names the handle, not the path.
        let request = json!({ "jobId": "delegation-new" });
        let shown = present_response(
            "workspace-provision",
            &request,
            json!({ "provisioned": true, "workspacePath": path }),
        );
        assert_eq!(
            shown["workspacePath"],
            "connection-workspace:delegation-new"
        );

        // The handle resolves on this device for probe and submit.
        let probe = json!({ "workspacePath": "connection-workspace:delegation-new" });
        let admitted = admit_call(store, &fixture.grants, "probe", probe.clone(), "conn-1")
            .await
            .unwrap();
        assert_eq!(admitted["workspacePath"], path.as_str());
        let answer = present_response(
            "probe",
            &probe,
            json!({ "workspace": { "path": path, "isGitRepository": true } }),
        );
        assert_eq!(
            answer["workspace"]["path"],
            "connection-workspace:delegation-new"
        );
        assert_eq!(answer["workspace"]["isGitRepository"], true);
        assert!(
            admit_call(store, &fixture.grants, "probe", probe.clone(), "conn-2")
                .await
                .is_err()
        );

        let submit = json!({
            "jobId": "delegation-new",
            "workspacePath": "connection-workspace:delegation-new",
        });
        let admitted = admit_call(store, &fixture.grants, "submit", submit, "conn-1")
            .await
            .unwrap();
        assert_eq!(admitted["workspacePath"], path.as_str());
        ensure_provisioned_workspace(store, "delegation-new", "conn-1", &path).unwrap();
        // A handle of another job is refused.
        let other = json!({
            "jobId": "delegation-other",
            "workspacePath": "connection-workspace:delegation-new",
        });
        assert!(
            admit_call(store, &fixture.grants, "submit", other, "conn-1")
                .await
                .is_err()
        );
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
        for verb in ["answer", "workspace-sync", "unknown"] {
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
    }
}
