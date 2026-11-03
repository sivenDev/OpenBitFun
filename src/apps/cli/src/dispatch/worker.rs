use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use openbitfun_agent_runtime::sdk::{
    AgentDialogSteerRequest, AgentDialogTurnRequest, AgentSessionCreateRequest,
    AgentSessionModelSelection, AgentSessionModelSelectionUpdateRequest,
    AgentSessionRestoreRequest, AgentTurnCancellationRequest, AgentTurnSettlementRequest,
    PermissionReply, PermissionReplySource, PermissionRequest, PermissionRequestEvent,
};
use openbitfun_agent_runtime::thread_goal::{ThreadGoalRunDisposition, ThreadGoalRunTracker};
use openbitfun_events::{project_agentic_frontend_event, AgenticEvent};
use openbitfun_runtime_ports::{
    AgentSubmissionSource, DialogSubmissionPolicy, SessionExecutionTarget,
};

use crate::{shutdown_mcp_servers, BootstrapProfile};

use super::permissions::{self, REJECT_AND_REPORT_REASON};
use super::protocol::{DispatchApprovalPolicy, DispatchEvent, DispatchJobState, DispatchTurnKind};
use super::store::{DispatchStore, WorkspaceLock};
use super::{audit, share_guard};

const TURN_SETTLEMENT_TIMEOUT_MS: u64 = 5_000;

pub(crate) async fn run(job_id: String) -> Result<()> {
    let store = DispatchStore::open_default()?;
    // Decided once, before the runtime starts. A shared assistant may hand
    // work to external agents, whose prompts reach the owner through the
    // job's approval mailbox. Any other job has no guaranteed controller to
    // answer an external agent's own prompts, so external agents fail there
    // with a reason.
    let job = store.load_job(&job_id).ok();
    if !job.as_ref().is_some_and(runs_assistant) {
        crate::agent::agentic_system::disable_external_agents_for_process(
            crate::agent::agentic_system::DISPATCH_EXTERNAL_AGENTS_UNSUPPORTED,
        );
    }
    let Some(_worker_lease) = store.try_acquire_worker_lease(&job_id)? else {
        // A retry may briefly spawn a duplicate after its controller crashes.
        // Only the lease holder may publish a PID or execute the job.
        return Ok(());
    };
    let worker_pid = std::process::id();
    store.write_pid(&job_id, worker_pid)?;
    store.clear_preparing(&job_id);
    let result = run_inner(&store, &job_id).await;
    if let Err(error) = &result {
        let _ = store.mark_state(
            &job_id,
            DispatchJobState::Failed,
            None,
            Some(format!("{error:#}")),
        );
    }
    if let (Some(job), Ok(state)) = (job.as_ref(), store.load_state(&job_id)) {
        if state.state.is_terminal() {
            audit::record(
                &job.request,
                openbitfun_core::service::delegation_audit::DelegationAuditEvent::Finished,
                Some(state.state),
                state.last_error.as_deref(),
                external_agents_used(),
            )
            .await;
        }
    }
    store.clear_pending_permissions(&job_id);
    store.clear_preparing(&job_id);
    store.clear_hold(&job_id);
    store.remove_pid_if_matches(&job_id, worker_pid);
    shutdown_mcp_servers().await;
    result
}

async fn run_inner(store: &DispatchStore, job_id: &str) -> Result<()> {
    let job = store.load_job(job_id)?;
    let state = store.load_state(job_id)?;
    if state.state.is_terminal() {
        return Ok(());
    }
    if state.cancel_requested() {
        store.mark_state(
            job_id,
            DispatchJobState::Cancelled,
            state.turn_id.as_deref(),
            Some("Dispatch worker observed a cancellation request".to_string()),
        )?;
        return Ok(());
    }
    if state.turn_id.is_some() {
        bail!(
            "dispatch worker cannot replay an already-submitted turn after process loss; session {} remains available on the target",
            job.request.session_id
        );
    }

    let workspace = Path::new(&job.request.workspace_path);
    if !workspace.is_absolute() {
        bail!("dispatch workspacePath must be absolute");
    }
    if !workspace.is_dir() {
        bail!(
            "dispatch workspace does not exist or is not a directory: {}",
            workspace.display()
        );
    }
    // Per-turn overrides are read before runtime bootstrap because the
    // approval policy is baked into initialize_core_services. Nothing can
    // enqueue another turn between this peek and the claim below: queueing
    // requires a terminal state and the job is already non-terminal here.
    let pending_turn = store.peek_follow_up_turn(job_id)?;
    let effective_model = pending_turn
        .as_ref()
        .and_then(|turn| turn.model.clone())
        .or_else(|| job.request.model.clone());
    let effective_reasoning_preset = pending_turn
        .as_ref()
        .and_then(|turn| turn.reasoning_preset.clone())
        .or_else(|| job.request.reasoning_preset.clone());
    let effective_policy = pending_turn
        .as_ref()
        .and_then(|turn| turn.approval_policy)
        .unwrap_or(job.request.approval_policy);
    super::ensure_selected_model_ready(effective_model.as_deref()).await?;
    if let Some(model) = effective_model.as_deref() {
        super::validate_reasoning_preset(model, effective_reasoning_preset.as_deref()).await?;
    }

    // Every detached worker takes the same stable lock for a canonical target
    // workspace. Waiting workers remain Queued and are visible/cancellable.
    // Tasks for a shared assistant are the exception: they are message tasks
    // in their device's tenant workspace, each in its own session, so one
    // device's tasks run side by side up to the share's limit.
    let _workspace_lock = if job.request.delegation.is_some() {
        None
    } else {
        let lock_path = store.workspace_lock_path(&job.request.workspace_path);
        Some(WorkspaceLock::acquire(&lock_path)?)
    };
    // A task over its share's limit stays Queued here until a place is free;
    // the admission lock is held until the job is marked running.
    let place = if job.request.delegation.is_some() {
        share_guard::wait_for_place(store, &super::delegation::share_store()?, job_id).await?
    } else {
        None
    };
    let state = store.load_state(job_id)?;
    if state.state.is_terminal() {
        return Ok(());
    }
    if state.cancel_requested() {
        store.mark_state(
            job_id,
            DispatchJobState::Cancelled,
            state.turn_id.as_deref(),
            Some("Dispatch worker observed a cancellation request".to_string()),
        )?;
        return Ok(());
    }
    store.mark_state(job_id, DispatchJobState::Running, None, None)?;
    drop(place);

    // Persist the effective options before execution so `list`/`status` and
    // any replacement worker observe the same choices this turn runs with.
    let (model_changed, _, policy_changed) = store.update_job_request_options(
        job_id,
        effective_model.as_deref(),
        effective_reasoning_preset.as_deref(),
        effective_policy,
    )?;
    if policy_changed {
        store.append_event(
            job_id,
            &DispatchEvent::approval_policy_selected(effective_policy),
        )?;
    }
    if model_changed {
        store.append_event(
            job_id,
            &DispatchEvent::model_selected(effective_model.as_deref()),
        )?;
    }

    let runtime = crate::initialize_core_services(
        workspace,
        permissions::cli_policy(effective_policy),
        BootstrapProfile::Execution,
    )
    .await?;
    let agent_runtime = runtime.agent_runtime().clone();
    let compatibility = runtime.compatibility().clone();
    let mut event_rx = agent_runtime
        .subscribe_events()
        .map_err(|error| anyhow!(error.into_message()))?;
    let mut permission_rx = agent_runtime
        .subscribe_permission_requests()
        .map_err(|error| anyhow!(error.into_message()))?;

    let workspace_path = job.request.workspace_path.clone();
    // A follow-up turn runs against the session the previous turn built, so its
    // history is the agent's context. Restoring first is what makes a dispatch
    // session a conversation; creating unconditionally would fail on the second
    // turn because the persisted id already exists.
    let restore_error = agent_runtime
        .restore_session(AgentSessionRestoreRequest {
            workspace_id: Some(runtime.workspace().id.clone()),
            workspace_path: String::new(),
            session_id: job.request.session_id.clone(),
            include_internal: false,
            remote_connection_id: None,
            remote_ssh_host: None,
        })
        .await
        .err();
    if let Some(error) = restore_error.as_ref() {
        // Expected on the first turn — there is nothing to restore yet.
        tracing::debug!("Dispatch session restore did not apply: {error}");
    }
    if restore_error.is_some() {
        agent_runtime
            .create_session_with_id(
                job.request.session_id.clone(),
                AgentSessionCreateRequest {
                    session_name: job.title.clone(),
                    agent_type: session_agent_type(&job.request).to_string(),
                    agent_route_key: None,
                    workspace_path: Some(workspace_path.clone()),
                    project_workspace_path: Some(workspace_path.clone()),
                    execution_target: Some(SessionExecutionTarget::local(workspace_path.clone())),
                    workspace_id: Some(runtime.workspace().id.clone()),
                    remote_connection_id: None,
                    remote_ssh_host: None,
                    model_id: effective_model.clone(),
                    metadata: serde_json::Map::new(),
                },
            )
            .await
            .map_err(|error| anyhow!(error.into_message()))
            // A follow-up turn reaches this only when its restore failed, and
            // creating then fails on the existing persisted id. Carry the
            // restore error so the report names the real cause instead of the
            // "already exists" symptom.
            .with_context(|| match restore_error {
                Some(restore) => {
                    format!("create target-owned dispatch session after restore failed: {restore}")
                }
                None => "create target-owned dispatch session".to_string(),
            })?;
    }
    if let Some(model) = effective_model.clone() {
        let reasoning_preset = effective_reasoning_preset
            .as_deref()
            .filter(|preset| *preset != "auto")
            .map(str::to_string);
        agent_runtime
            .update_session_model_selection(AgentSessionModelSelectionUpdateRequest {
                session_id: job.request.session_id.clone(),
                selection: AgentSessionModelSelection {
                    model_id: model,
                    reasoning_preset,
                    routing_mode: None,
                },
            })
            .await
            .map_err(|error| anyhow!(error.into_message()))
            .context("apply dispatch turn model and reasoning preset")?;
    }

    let mut goal_run = ThreadGoalRunTracker::new(
        agent_runtime
            .get_thread_goal(openbitfun_agent_runtime::sdk::AgentThreadGoalGetRequest {
                session_id: job.request.session_id.clone(),
                workspace_path: workspace_path.clone(),
                remote_connection_id: None,
                remote_ssh_host: None,
            })
            .await
            .map_err(|error| anyhow!(error.into_message()))?,
    );
    let mut turn_id = uuid::Uuid::new_v4().to_string();
    // Claim the queued follow-up and persist the turn id in one step. A crash
    // after the Runtime accepts the turn must never make a replacement worker
    // submit the prompt a second time.
    let follow_up = store.claim_follow_up_turn(job_id, &turn_id)?;
    let turn_kind = follow_up.as_ref().map(|turn| turn.kind).unwrap_or_default();
    match turn_kind {
        DispatchTurnKind::Prompt => {
            let prompt = follow_up
                .as_ref()
                .map(|turn| turn.prompt.clone())
                .unwrap_or_else(|| job.request.prompt.clone());
            let turn_attachments = follow_up
                .as_ref()
                .map(|turn| runtime_attachments(&turn.attachments))
                .unwrap_or_else(|| runtime_attachments(&job.request.attachments));
            agent_runtime
                .submit_dialog_turn(AgentDialogTurnRequest {
                    session_id: job.request.session_id.clone(),
                    message: prompt,
                    output_schema: None,
                    original_message: None,
                    turn_id: Some(turn_id.clone()),
                    execution: Default::default(),
                    agent_type: session_agent_type(&job.request).to_string(),
                    workspace_path: Some(workspace_path),
                    workspace_id: Some(runtime.workspace().id.clone()),
                    remote_connection_id: None,
                    remote_ssh_host: None,
                    policy: DialogSubmissionPolicy::for_source(AgentSubmissionSource::Cli),
                    reply_route: None,
                    prepended_reminders: Vec::new(),
                    attachments: turn_attachments,
                    metadata: turn_metadata(
                        effective_policy,
                        &job.request,
                        delegation_trust(&job.request).await,
                    ),
                })
                .await
                .map_err(|error| anyhow!(error.into_message()))
                .context("submit dispatch dialog turn")?;
        }
        DispatchTurnKind::Compact => {
            // The compaction runs as a turn with this worker's turn id, so
            // its DialogTurn/ContextCompression events flow through the same
            // event loop and settle the job like any other turn.
            compatibility
                .start_manual_compaction(job.request.session_id.clone(), turn_id.clone())
                .await
                .map_err(anyhow::Error::msg)
                .context("start dispatch manual compaction")?;
        }
    }

    let mut event_scope = JobEventScope::new(job.request.session_id.clone(), turn_id.clone());
    let mut initial_permissions = agent_runtime
        .pending_permission_requests()
        .unwrap_or_default()
        .into_iter()
        .collect::<VecDeque<_>>();
    let mut handled_permissions = HashSet::new();
    let mut mailbox_tick = tokio::time::interval(Duration::from_millis(200));
    mailbox_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // A task from another device stops when the owner revokes or blocks
    // that device or stops sharing the assistant.
    let guard = share_guard_for(&job);
    let mut guard_tick = tokio::time::interval(share_guard::GUARD_INTERVAL);
    guard_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let (terminal_state, terminal_error) = loop {
        if let Some(request) = initial_permissions.pop_front() {
            if event_scope.permission_targets_job(&request)
                && handled_permissions.insert(request.request_id.clone())
            {
                if let Some(reason) = handle_permission(
                    store,
                    job_id,
                    &agent_runtime,
                    &job.request.session_id,
                    &turn_id,
                    request,
                    effective_policy,
                )
                .await?
                {
                    break (DispatchJobState::Failed, Some(reason));
                }
            }
            continue;
        }

        tokio::select! {
            received = event_rx.recv() => {
                let envelope = match received {
                    Ok(envelope) => envelope,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        cancel_turn(&agent_runtime, &job.request.session_id, &turn_id, "dispatch_event_stream_lagged").await;
                        break (
                            DispatchJobState::Failed,
                            Some(format!("dispatch event stream lost {skipped} events; the turn was cancelled")),
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        cancel_turn(&agent_runtime, &job.request.session_id, &turn_id, "dispatch_event_stream_closed").await;
                        break (
                            DispatchJobState::Failed,
                            Some("dispatch event stream closed before the turn settled".to_string()),
                        );
                    }
                };
                let mut goal_disposition = None;
                if let AgenticEvent::ThreadGoalUpdated { session_id: owner, goal } = &envelope.event {
                    if owner == &job.request.session_id {
                        goal_disposition = goal_run.observe_goal(goal.clone().map(serde_json::from_value).transpose()?);
                    }
                }
                if let AgenticEvent::DialogTurnStarted { session_id: owner, turn_id: next_turn, user_message_metadata, .. } = &envelope.event {
                    if owner == &job.request.session_id && goal_run.accept_continuation(user_message_metadata.as_ref()) {
                        store.advance_goal_turn(job_id, &turn_id, next_turn)?;
                        turn_id = next_turn.clone();
                        event_scope.turn_id = turn_id.clone();
                    }
                }
                if !event_scope.admit(&envelope.event) {
                    continue;
                }
                let projection = project_agentic_frontend_event(envelope.event.clone())
                    .map(|projected| (projected.event_name, projected.payload));
                let raw = serde_json::to_value(&envelope)
                    .context("serialize dispatch Agent event")?;
                store.append_event(
                    job_id,
                    &DispatchEvent::agent_event(raw, projection),
                )?;
                if let Some(disposition) = goal_disposition {
                    match disposition {
                        ThreadGoalRunDisposition::Complete => break (DispatchJobState::Succeeded, None),
                        ThreadGoalRunDisposition::Stopped(status) => break (DispatchJobState::Failed, Some(format!("Thread goal stopped with status {status:?}; the objective is not complete"))),
                        ThreadGoalRunDisposition::Continue => {}
                    }
                }
                if let Some(outcome) = terminal_outcome(&envelope.event, &turn_id) {
                    if outcome.0 == DispatchJobState::Succeeded && turn_kind == DispatchTurnKind::Prompt {
                        match goal_run.after_successful_turn() {
                            ThreadGoalRunDisposition::Continue => continue,
                            ThreadGoalRunDisposition::Complete => {}
                            ThreadGoalRunDisposition::Stopped(status) => break (DispatchJobState::Failed, Some(format!("Thread goal stopped with status {status:?}; the objective is not complete"))),
                        }
                    }
                    break outcome;
                }
            }
            received = permission_rx.recv() => {
                let event = match received {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        cancel_turn(&agent_runtime, &job.request.session_id, &turn_id, "dispatch_permission_stream_lagged").await;
                        break (
                            DispatchJobState::Failed,
                            Some(format!("dispatch permission stream lost {skipped} requests; the turn was cancelled")),
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        cancel_turn(&agent_runtime, &job.request.session_id, &turn_id, "dispatch_permission_stream_closed").await;
                        break (
                            DispatchJobState::Failed,
                            Some("dispatch permission stream closed before the turn settled".to_string()),
                        );
                    }
                };
                let PermissionRequestEvent::Asked { request } = event else {
                    continue;
                };
                if !event_scope.permission_targets_job(&request)
                    || !handled_permissions.insert(request.request_id.clone())
                {
                    continue;
                }
                if let Some(reason) = handle_permission(
                    store,
                    job_id,
                    &agent_runtime,
                    &job.request.session_id,
                    &turn_id,
                    request,
                    effective_policy,
                )
                .await?
                {
                    break (DispatchJobState::Failed, Some(reason));
                }
            }
            _ = guard_tick.tick(), if guard.is_some() => {
                let stop = match guard.as_ref() {
                    Some(guard) => guard.stop_reason().await,
                    None => None,
                };
                if let Some(reason) = stop {
                    cancel_turn(&agent_runtime, &job.request.session_id, &turn_id, "dispatch_share_guard").await;
                    break (DispatchJobState::Failed, Some(reason));
                }
            }
            _ = mailbox_tick.tick() => {
                if let Some(outcome) = process_mailboxes(
                    store,
                    job_id,
                    &agent_runtime,
                    &job.request.session_id,
                    &turn_id,
                ).await? {
                    break outcome;
                }
            }
        }
    };

    let settlement = agent_runtime
        .wait_for_turn_settlement(AgentTurnSettlementRequest {
            session_id: job.request.session_id.clone(),
            turn_id: turn_id.clone(),
            wait_timeout_ms: TURN_SETTLEMENT_TIMEOUT_MS,
        })
        .await;
    let (terminal_state, terminal_error) = match settlement {
        Ok(_) => (terminal_state, terminal_error),
        Err(error) => (
            DispatchJobState::Failed,
            Some(format!(
                "dispatch turn reached a terminal event but did not settle: {}",
                error.into_message()
            )),
        ),
    };
    store.mark_state(job_id, terminal_state, Some(&turn_id), terminal_error)?;
    store.clear_pending_permissions(job_id);
    Ok(())
}

fn share_guard_for(job: &super::store::DispatchJobRecord) -> Option<share_guard::ShareGuard> {
    job.request.source_share_key.as_ref()?;
    match super::delegation::share_store() {
        Ok(grants) => share_guard::ShareGuard::for_job(job, grants),
        Err(error) => {
            // Admission already checked the grant; the guard is a second
            // line, so it degrades to that check rather than failing the job.
            tracing::warn!("Share guard unavailable for a task from another device: {error:#}");
            None
        }
    }
}

async fn handle_permission(
    store: &DispatchStore,
    job_id: &str,
    runtime: &openbitfun_agent_runtime::sdk::AgentRuntime,
    session_id: &str,
    turn_id: &str,
    request: PermissionRequest,
    policy: DispatchApprovalPolicy,
) -> Result<Option<String>> {
    if policy == DispatchApprovalPolicy::Remote {
        store.save_pending_permission(job_id, &request)?;
        store.append_event(
            job_id,
            &DispatchEvent::permission_pending(&request.request_id),
        )?;
        return Ok(None);
    }
    let reason = match policy {
        DispatchApprovalPolicy::RejectAndReport => REJECT_AND_REPORT_REASON.to_string(),
        DispatchApprovalPolicy::Auto => format!(
            "Dispatch Auto policy could not safely auto-approve permission request {}",
            request.request_id
        ),
        DispatchApprovalPolicy::Remote => unreachable!("remote handled above"),
    };
    store.append_event(
        job_id,
        &DispatchEvent::permission_rejected(
            serde_json::to_value(&request).context("serialize permission request")?,
            reason.clone(),
        ),
    )?;
    runtime
        .respond_permission_with_source(
            &request.request_id,
            PermissionReply::Reject {
                feedback: Some(reason.clone()),
            },
            PermissionReplySource::System,
        )
        .await
        .map_err(|error| anyhow!(error.into_message()))
        .context("reject unattended dispatch permission")?;
    cancel_turn(runtime, session_id, turn_id, "dispatch_permission_rejected").await;
    Ok(Some(reason))
}

async fn process_mailboxes(
    store: &DispatchStore,
    job_id: &str,
    runtime: &openbitfun_agent_runtime::sdk::AgentRuntime,
    session_id: &str,
    turn_id: &str,
) -> Result<Option<(DispatchJobState, Option<String>)>> {
    let state = store.load_state(job_id)?;
    if state.cancel_requested() {
        cancel_turn(runtime, session_id, turn_id, "dispatch_cancel_requested").await;
        return Ok(Some((
            DispatchJobState::Cancelled,
            Some("Dispatch worker observed a cancellation request".to_string()),
        )));
    }

    for answer in store.list_permission_answers(job_id)? {
        runtime
            .respond_permission_with_source(
                &answer.request_id,
                answer.reply.clone(),
                PermissionReplySource::User,
            )
            .await
            .map_err(|error| anyhow!(error.into_message()))
            .with_context(|| {
                format!(
                    "apply remote dispatch permission response {}",
                    answer.request_id
                )
            })?;
        store.mark_permission_resolved(job_id, &answer)?;
        store.append_event(
            job_id,
            &DispatchEvent::permission_resolved(&answer.request_id),
        )?;
    }

    for request in store.list_pending_append_messages(job_id)? {
        runtime
            .steer_dialog_turn(AgentDialogSteerRequest {
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                content: request.content.clone(),
                display_content: request.display_content.clone(),
                attachments: runtime_attachments(&request.attachments),
                metadata: serde_json::Map::new(),
            })
            .await
            .map_err(|error| anyhow!(error.into_message()))
            .with_context(|| {
                format!(
                    "append message {} to running dispatch turn",
                    request.message_id
                )
            })?;
        store.mark_append_message_consumed(job_id, &request)?;
        store.append_event(
            job_id,
            &DispatchEvent::message_appended(&request.message_id),
        )?;
    }
    Ok(None)
}

async fn cancel_turn(
    runtime: &openbitfun_agent_runtime::sdk::AgentRuntime,
    session_id: &str,
    turn_id: &str,
    reason: &str,
) {
    if let Err(error) = runtime
        .cancel_turn(AgentTurnCancellationRequest {
            session_id: session_id.to_string(),
            turn_id: Some(turn_id.to_string()),
            source: Some(AgentSubmissionSource::Cli),
            requester_session_id: None,
            reason: Some(reason.to_string()),
            wait_timeout_ms: None,
            cancel_descendants: true,
        })
        .await
    {
        tracing::error!("Failed to cancel dispatch turn: {}", error.into_message());
    }
}

fn runtime_attachments(
    attachments: &[super::protocol::DispatchAttachment],
) -> Vec<openbitfun_runtime_ports::AgentInputAttachment> {
    attachments
        .iter()
        .map(|attachment| {
            openbitfun_runtime_ports::AgentInputAttachment::remote_image(
                attachment.id.clone(),
                attachment
                    .name
                    .clone()
                    .unwrap_or_else(|| attachment.id.clone()),
                attachment.data_url.clone(),
            )
        })
        .collect()
}

/// Which sessions' events belong in this job's log.
///
/// The job session's turn-scoped events must match the worker's turn, and any
/// subagent session linked under it (recursively) is admitted wholesale so
/// the controller can project child transcripts.
struct JobEventScope {
    session_id: String,
    turn_id: String,
    children: std::collections::HashSet<String>,
}

impl JobEventScope {
    fn new(session_id: String, turn_id: String) -> Self {
        Self {
            session_id,
            turn_id,
            children: std::collections::HashSet::new(),
        }
    }

    fn admit(&mut self, event: &AgenticEvent) -> bool {
        if let AgenticEvent::SubagentSessionLinked {
            session_id: child_session,
            parent_session_id,
            agent_type,
            ..
        } = event
        {
            if parent_session_id == &self.session_id || self.children.contains(parent_session_id) {
                note_external_agent(agent_type.as_deref());
                self.children.insert(child_session.clone());
                return true;
            }
            return false;
        }
        match event.session_id() {
            Some(event_session) if event_session == self.session_id => {
                event_turn_id(event).is_none_or(|event_turn| event_turn == self.turn_id)
            }
            Some(event_session) => self.children.contains(event_session),
            None => true,
        }
    }

    fn permission_targets_job(&self, request: &PermissionRequest) -> bool {
        if crate::runtime::approval::permission_request_targets_session(request, &self.session_id) {
            return true;
        }
        self.children.iter().any(|child| {
            crate::runtime::approval::permission_request_targets_session(request, child)
        })
    }
}

fn event_turn_id(event: &AgenticEvent) -> Option<&str> {
    match event {
        AgenticEvent::DialogTurnStarted { turn_id, .. }
        | AgenticEvent::DialogTurnCompleted { turn_id, .. }
        | AgenticEvent::DialogTurnCancelled { turn_id, .. }
        | AgenticEvent::DialogTurnFailed { turn_id, .. }
        | AgenticEvent::TokenUsageUpdated { turn_id, .. }
        | AgenticEvent::ContextCompressionStarted { turn_id, .. }
        | AgenticEvent::ContextCompressionCompleted { turn_id, .. }
        | AgenticEvent::ContextCompressionFailed { turn_id, .. }
        | AgenticEvent::ModelRoundStarted { turn_id, .. }
        | AgenticEvent::ModelRoundCompleted { turn_id, .. }
        | AgenticEvent::TextChunk { turn_id, .. }
        | AgenticEvent::ThinkingChunk { turn_id, .. }
        | AgenticEvent::ToolEvent { turn_id, .. }
        | AgenticEvent::DeepReviewQueueStateChanged { turn_id, .. }
        | AgenticEvent::UserSteeringInjected { turn_id, .. } => Some(turn_id),
        _ => None,
    }
}

fn terminal_outcome(
    event: &AgenticEvent,
    turn_id: &str,
) -> Option<(DispatchJobState, Option<String>)> {
    match event {
        AgenticEvent::DialogTurnCompleted {
            turn_id: event_turn,
            success,
            finish_reason,
            has_final_response,
            ..
        } if event_turn == turn_id => {
            if *success == Some(false) {
                let reason = finish_reason
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("unsuccessful_completion");
                let detail = if *has_final_response == Some(false) {
                    format!("Dispatch completed without a successful final response: {reason}")
                } else {
                    format!("Dispatch completed unsuccessfully: {reason}")
                };
                Some((DispatchJobState::Failed, Some(detail)))
            } else {
                Some((DispatchJobState::Succeeded, None))
            }
        }
        AgenticEvent::DialogTurnFailed {
            turn_id: event_turn,
            error,
            ..
        } if event_turn == turn_id => Some((DispatchJobState::Failed, Some(error.clone()))),
        AgenticEvent::DialogTurnCancelled {
            turn_id: event_turn,
            ..
        } if event_turn == turn_id => Some((
            DispatchJobState::Cancelled,
            Some("Dispatch turn was cancelled".to_string()),
        )),
        AgenticEvent::SystemError { error, .. } => {
            Some((DispatchJobState::Failed, Some(error.clone())))
        }
        _ => None,
    }
}

/// Turn context for a dispatch job. A delegated job also carries who asked for
/// it, so the turn can tell it was delegated and must not delegate onward.
fn turn_metadata(
    policy: super::protocol::DispatchApprovalPolicy,
    request: &super::protocol::DispatchSubmitRequest,
    trust: DelegationTrust,
) -> serde_json::Map<String, serde_json::Value> {
    let mut metadata = permissions::metadata(policy);
    if let Some(delegation) = &request.delegation {
        let mut facts = serde_json::json!({
            "assistantId": delegation.assistant_id,
            "sourceUser": request.source_user,
            "source": delegation.source,
        });
        if let Some(object) = facts.as_object_mut() {
            object.extend(DelegatedTurnFacts::metadata_fields(
                request.assistant_workspace.as_deref(),
                &request.workspace_path,
                trust,
                &delegation.call_chain,
            ));
        }
        metadata.insert(DISPATCH_DELEGATION_CONTEXT_KEY.to_string(), facts);
    }
    metadata
}

/// How far the owner trusts the users of the shared assistant `request`
/// runs, read when each turn starts so a change applies from the next turn.
pub(super) async fn delegation_trust(
    request: &super::protocol::DispatchSubmitRequest,
) -> DelegationTrust {
    match super::delegation::share_store() {
        Ok(store) => delegation_trust_in(request, &store).await,
        Err(_) => DelegationTrust::Normal,
    }
}

/// Normal unless the share of `request` is readable and set to full trust.
async fn delegation_trust_in(
    request: &super::protocol::DispatchSubmitRequest,
    store: &openbitfun_core::service::lan_share::SharedAssistantStore,
) -> DelegationTrust {
    let Some(delegation) = &request.delegation else {
        return DelegationTrust::Normal;
    };
    match store.share_of(&delegation.assistant_id).await {
        Ok(Some(share)) => share.settings.trust,
        _ => DelegationTrust::Normal,
    }
}

/// External agents this worker's job started, for its audit entry. One
/// worker process runs one job.
static EXTERNAL_AGENTS_USED: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

fn note_external_agent(agent_type: Option<&str>) {
    if let Some(agent_type) = agent_type.filter(|agent_type| agent_type.starts_with("acp:")) {
        if let Ok(mut used) = EXTERNAL_AGENTS_USED.lock() {
            used.insert(agent_type.to_string());
        }
    }
}

fn external_agents_used() -> Vec<String> {
    EXTERNAL_AGENTS_USED
        .lock()
        .map(|used| used.iter().cloned().collect())
        .unwrap_or_default()
}

/// Whether `job` runs as a shared assistant, which this device admitted.
fn runs_assistant(job: &super::store::DispatchJobRecord) -> bool {
    job.request.delegation.is_some() && job.request.assistant_workspace.is_some()
}

/// The agent mode the job's session runs: a shared assistant runs as the
/// assistant mode with its persona; any other job runs the requested agent.
fn session_agent_type(request: &super::protocol::DispatchSubmitRequest) -> &str {
    if request.delegation.is_some() && request.assistant_workspace.is_some() {
        ASSISTANT_RUN_AGENT_TYPE
    } else {
        &request.agent_type
    }
}

/// Turn context key naming the device that delegated a dispatch job. The
/// kernel reads it to refuse delegating such a turn onward.
pub(crate) use openbitfun_core::service::delegation::DISPATCH_DELEGATION_CONTEXT_KEY;
use openbitfun_core::service::delegation::{
    DelegatedTurnFacts, DelegationTrust, ASSISTANT_RUN_AGENT_TYPE,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn job(agent_type: &str, delegated: bool) -> super::super::store::DispatchJobRecord {
        use super::super::protocol::{
            DispatchDelegation, DispatchDelegationSource, DispatchSubmitRequest,
        };
        super::super::store::DispatchJobRecord {
            protocol_version:
                openbitfun_services_core::dispatch_contract::DISPATCH_PROTOCOL_VERSION,
            intent_sha256: String::new(),
            request: DispatchSubmitRequest {
                protocol_version:
                    openbitfun_services_core::dispatch_contract::DISPATCH_PROTOCOL_VERSION,
                job_id: "job-1".to_string(),
                session_id: "session-1".to_string(),
                workspace_path: "/tmp/job".to_string(),
                agent_type: agent_type.to_string(),
                prompt: "What changed?".to_string(),
                approval_policy: DispatchApprovalPolicy::Remote,
                model: None,
                reasoning_preset: None,
                title: None,
                attachments: Vec::new(),
                setup_audit: Vec::new(),
                delegation: delegated.then(|| DispatchDelegation {
                    assistant_id: "workspace".to_string(),
                    source: DispatchDelegationSource::default(),
                    call_chain: Vec::new(),
                }),
                source_share_key: delegated.then(|| "ab".repeat(32)),
                source_user: None,
                assistant_workspace: None,
            },
            created_at: String::new(),
            title: String::new(),
        }
    }

    #[tokio::test]
    async fn trust_follows_the_owners_choice_for_the_share() {
        use openbitfun_core::service::lan_share::{
            ShareConfirmations, ShareSettings, SharedAssistantStore,
        };
        let dir = tempfile::tempdir().unwrap();
        let store = SharedAssistantStore::new(dir.path().join("shared-assistants.json"));
        let request = job("assistant:workspace", true).request;
        // A share that does not exist, or cannot be read, trusts normally.
        assert_eq!(
            delegation_trust_in(&request, &store).await,
            DelegationTrust::Normal
        );
        let assistants = dir.path().join("personal_assistant");
        std::fs::create_dir_all(assistants.join("workspace")).unwrap();
        store
            .share(
                &assistants,
                "workspace",
                ShareSettings::default(),
                ShareConfirmations::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            delegation_trust_in(&request, &store).await,
            DelegationTrust::Normal
        );
        store
            .change_settings(
                "workspace",
                ShareSettings {
                    trust: DelegationTrust::Full,
                    ..ShareSettings::default()
                },
                ShareConfirmations {
                    full_trust_risk: true,
                    ..ShareConfirmations::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            delegation_trust_in(&request, &store).await,
            DelegationTrust::Full
        );
        std::fs::write(store.path(), b"{ not json").unwrap();
        assert_eq!(
            delegation_trust_in(&request, &store).await,
            DelegationTrust::Normal
        );
    }

    #[test]
    fn assistant_jobs_run_as_the_assistant_with_their_facts() {
        let mut assistant = job("assistant:workspace-ann", true);
        assistant.request.assistant_workspace =
            Some("/home/a/.openbitfun/personal_assistant/workspace-ann".to_string());
        assert!(runs_assistant(&assistant));
        assert_eq!(
            session_agent_type(&assistant.request),
            ASSISTANT_RUN_AGENT_TYPE
        );
        let metadata = turn_metadata(
            DispatchApprovalPolicy::Remote,
            &assistant.request,
            DelegationTrust::Full,
        );
        let mut context = std::collections::HashMap::new();
        DelegatedTurnFacts::copy_metadata_to_context(
            Some(&serde_json::Value::Object(metadata)),
            &mut context,
        );
        let facts = DelegatedTurnFacts::from_context(&context).expect("delegated facts");
        assert_eq!(
            facts.assistant_workspace.as_deref(),
            Some(std::path::Path::new(
                "/home/a/.openbitfun/personal_assistant/workspace-ann"
            ))
        );
        assert_eq!(
            facts.tenant_workspace.as_deref(),
            Some(std::path::Path::new("/tmp/job"))
        );
        assert_eq!(facts.trust, DelegationTrust::Full);

        // Other jobs keep their agent and have no assistant.
        let plain = job("Standard", true);
        assert!(!runs_assistant(&plain));
        assert_eq!(session_agent_type(&plain.request), "Standard");
    }

    #[test]
    fn dispatch_append_reuses_the_runtime_steering_port() {
        let source = include_str!("worker.rs").replace("\r\n", "\n");
        let mailboxes = source
            .split_once("async fn process_mailboxes(")
            .expect("mailbox processor")
            .1
            .split_once("async fn cancel_turn(")
            .expect("mailbox processor boundary")
            .0;

        assert!(mailboxes.contains(".steer_dialog_turn(AgentDialogSteerRequest"));
        assert!(!mailboxes.contains("compatibility.submit_steering"));
    }

    #[test]
    fn terminal_events_map_to_persistent_job_states() {
        let completed = AgenticEvent::DialogTurnCompleted {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            total_rounds: 1,
            total_tools: 0,
            duration_ms: 10,
            partial_recovery_reason: None,
            success: Some(true),
            finish_reason: Some("stop".to_string()),
            has_final_response: Some(true),
            background_delivery: None,
        };
        assert_eq!(
            terminal_outcome(&completed, "turn-1"),
            Some((DispatchJobState::Succeeded, None))
        );

        let failed = AgenticEvent::DialogTurnFailed {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            error: "model failed".to_string(),
            error_category: None,
            error_detail: None,
        };
        assert_eq!(
            terminal_outcome(&failed, "turn-1"),
            Some((DispatchJobState::Failed, Some("model failed".to_string())))
        );
    }

    #[test]
    fn linked_subagent_sessions_flow_into_the_job_event_scope() {
        let mut scope = JobEventScope::new("session-1".to_string(), "turn-1".to_string());

        let child_chunk = AgenticEvent::TextChunk {
            session_id: "child-session".to_string(),
            turn_id: "child-turn".to_string(),
            round_id: "round-1".to_string(),
            attempt_id: None,
            attempt_index: None,
            text: "child output".to_string(),
        };
        // A child that was never linked stays outside the scope.
        assert!(!scope.admit(&child_chunk));

        let linked = AgenticEvent::SubagentSessionLinked {
            session_id: "child-session".to_string(),
            subagent_dialog_turn_id: "child-turn".to_string(),
            parent_session_id: "session-1".to_string(),
            parent_dialog_turn_id: "turn-1".to_string(),
            parent_tool_call_id: "tool-1".to_string(),
            agent_type: Some("GeneralPurpose".to_string()),
            model_id: None,
            focused_review_display_label: None,
            continuation_policy: None,
        };
        assert!(scope.admit(&linked));
        assert!(scope.admit(&child_chunk));

        // Grandchildren link recursively through an admitted child.
        let grandchild_link = AgenticEvent::SubagentSessionLinked {
            session_id: "grandchild-session".to_string(),
            subagent_dialog_turn_id: "grandchild-turn".to_string(),
            parent_session_id: "child-session".to_string(),
            parent_dialog_turn_id: "child-turn".to_string(),
            parent_tool_call_id: "tool-2".to_string(),
            agent_type: None,
            model_id: None,
            focused_review_display_label: None,
            continuation_policy: None,
        };
        assert!(scope.admit(&grandchild_link));

        // A link from an unrelated parent is refused.
        let foreign_link = AgenticEvent::SubagentSessionLinked {
            session_id: "other-child".to_string(),
            subagent_dialog_turn_id: "t".to_string(),
            parent_session_id: "unrelated-session".to_string(),
            parent_dialog_turn_id: "t".to_string(),
            parent_tool_call_id: "tool-3".to_string(),
            agent_type: None,
            model_id: None,
            focused_review_display_label: None,
            continuation_policy: None,
        };
        assert!(!scope.admit(&foreign_link));

        // Parent turn discipline is unchanged.
        let parent_chunk = AgenticEvent::TextChunk {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            round_id: "round-1".to_string(),
            attempt_id: None,
            attempt_index: None,
            text: "parent output".to_string(),
        };
        assert!(scope.admit(&parent_chunk));
        let stale_parent_chunk = AgenticEvent::TextChunk {
            session_id: "session-1".to_string(),
            turn_id: "turn-0".to_string(),
            round_id: "round-1".to_string(),
            attempt_id: None,
            attempt_index: None,
            text: "stale output".to_string(),
        };
        assert!(!scope.admit(&stale_parent_chunk));
    }
}
