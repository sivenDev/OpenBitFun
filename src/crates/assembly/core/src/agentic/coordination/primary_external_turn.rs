//! Top-level turns of an `acp:` external agent.
//!
//! Interactive hosts run external agents only as delegated child sessions
//! (`AgentSpawn`/`Task`). A Dispatch worker that runs a job delegated to an
//! opened `acp:` agent has no parent session: the job session itself is the
//! external agent's conversation. Such a host calls
//! [`ConversationCoordinator::allow_primary_external_agent_turns`]; every
//! other host keeps rejecting `acp:` as a session mode.
//!
//! The turn keeps the coordinator's lifecycle (turn record, Processing state,
//! settlement, `cancel_dialog_turn`, terminal events) but skips the native
//! prompt, tool, and model preparation: the external agent brings its own.
//! Its permission prompts go through the same route and effective policy as
//! a delegated child, so a supervisor of the session (such as the Dispatch
//! approval mailbox) answers them.

use super::*;

impl ConversationCoordinator {
    /// Lets top-level sessions of this process run `acp:` agents. Call once,
    /// before the first turn; there is no way back.
    pub fn allow_primary_external_agent_turns(&self) {
        self.primary_external_agent_turns
            .store(true, Ordering::SeqCst);
    }

    pub(super) fn primary_external_agent_client_id<'a>(
        &self,
        session: &Session,
        agent_type: &'a str,
    ) -> Option<&'a str> {
        if session.kind == SessionKind::Subagent
            || !self.primary_external_agent_turns.load(Ordering::SeqCst)
        {
            return None;
        }
        external_agent_client_id(agent_type)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn start_primary_external_agent_turn(
        &self,
        session: Session,
        session_workspace: Option<WorkspaceBinding>,
        agent_type: String,
        client_id: String,
        user_input: String,
        turn_id: Option<String>,
        user_message_metadata: Option<serde_json::Value>,
    ) -> OpenBitFunResult<()> {
        let session_id = session.session_id.clone();
        let context = external_agent_turn_context(
            &agent_type,
            &session_id,
            session.config.is_remote_workspace(),
            session_workspace.as_ref(),
        )?;
        let context_vars = context_vars_from_metadata(user_message_metadata.as_ref());
        let policy = match ExternalAgentPermissionPolicy::resolve(
            &agent_type,
            &session_id,
            session_workspace.as_ref(),
            None,
            &context_vars,
            None,
        )
        .await
        {
            Ok(policy) => Some(policy),
            Err(error) => {
                warn!(
                    "External agent permission policy unavailable; its prompts will be rejected: agent_type={}, error={}",
                    agent_type, error
                );
                None
            }
        };
        let permissions = ExternalAgentPermissionRoute::for_workspace(
            self.tool_pipeline.permission_request_manager(),
            session_workspace.as_ref(),
            None,
            policy,
        );

        let turn_index = self.session_manager.get_turn_count(&session_id);
        let admission_facts = TurnAdmissionSessionFacts::from_session(&session);
        // Sets the session to Processing for this turn.
        let turn_id = self
            .session_manager
            .start_dialog_turn_with_prepended_messages_if_session_matches(
                &session_id,
                agent_type.clone(),
                user_input.clone(),
                turn_id,
                None,
                Vec::new(),
                user_message_metadata.clone(),
                &admission_facts,
            )
            .await?;

        let active_counter = self
            .active_turns_per_session
            .entry(session_id.clone())
            .or_insert_with(|| Arc::new(AtomicUsize::new(0)))
            .clone();
        active_counter.fetch_add(1, Ordering::SeqCst);
        let settlement = self
            .turn_settlements
            .register_accepted(session_id.clone(), turn_id.clone());
        // `cancel_dialog_turn` signals the token registered for the turn.
        let cancel_token = CancellationToken::new();
        self.execution_engine
            .register_cancel_token(&turn_id, cancel_token.clone());

        self.emit_event(AgenticEvent::DialogTurnStarted {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            turn_index,
            user_input: user_input.clone(),
            original_user_input: None,
            user_message_metadata: user_message_metadata.clone(),
        })
        .await;
        info!(
            "External agent turn started: session_id={}, turn_id={}, agent_type={}",
            session_id, turn_id, agent_type
        );

        let turn = ExternalAgentTurn {
            executor: self.external_agent_executor(),
            event_queue: self.event_queue.clone(),
            agent_type: agent_type.clone(),
            dialog_turn_id: turn_id.clone(),
            request: openbitfun_runtime_ports::ExternalAgentTurnRequest {
                client_id,
                context,
                prompt: user_input.clone(),
                timeout_seconds: None,
            },
            permissions,
            cancel_token,
        };
        let session_manager = self.session_manager.clone();
        let execution_engine = self.execution_engine.clone();
        let event_queue = self.event_queue.clone();
        let scheduler_notify_tx = self.scheduler_notify_tx.get().cloned();
        let workspace_path = session_workspace
            .as_ref()
            .map(|workspace| workspace.root_path_string());
        let storage_path = session_workspace
            .as_ref()
            .map(|workspace| workspace.session_storage_dir().to_path_buf());
        tokio::spawn(async move {
            let _settlement = settlement;
            let status = match run_external_agent_turn(turn).await {
                Ok(result) => {
                    Self::emit_outside_engine_turn_completed(
                        event_queue.as_ref(),
                        session_manager.as_ref(),
                        &session_id,
                        &turn_id,
                        &result,
                    )
                    .await;
                    Self::persist_completed_dialog_turn(
                        event_queue.as_ref(),
                        session_manager.as_ref(),
                        scheduler_notify_tx.as_ref(),
                        &session_id,
                        &turn_id,
                        &result,
                        None,
                    )
                    .await
                    .0
                }
                Err(OpenBitFunError::Cancelled(_)) => {
                    Self::persist_cancelled_dialog_turn_with_messages(
                        event_queue.as_ref(),
                        session_manager.as_ref(),
                        scheduler_notify_tx.as_ref(),
                        &session_id,
                        &turn_id,
                        true,
                        &[],
                    )
                    .await
                }
                Err(error) => {
                    warn!(
                        "External agent turn failed: session_id={}, turn_id={}, error={}",
                        session_id, turn_id, error
                    );
                    Self::persist_failed_dialog_turn(
                        event_queue.as_ref(),
                        session_manager.as_ref(),
                        scheduler_notify_tx.as_ref(),
                        &session_id,
                        &turn_id,
                        &error,
                        true,
                    )
                    .await
                }
            };
            execution_engine.cleanup_cancel_token(&turn_id).await;
            Self::finalize_persisted_turn_in_workspace_if_needed(
                session_manager.as_ref(),
                &session_id,
                &turn_id,
                turn_index,
                &agent_type,
                &user_input,
                workspace_path.as_deref(),
                storage_path.as_deref(),
                Some(status),
                user_message_metadata,
            )
            .await;
            session_manager.reset_session_state_if_processing(&session_id, &turn_id);
            active_counter.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(())
    }
}

/// The string-valued turn metadata (permission mode, auto-approval) as the
/// context variables the permission policy reads.
fn context_vars_from_metadata(metadata: Option<&serde_json::Value>) -> HashMap<String, String> {
    metadata
        .and_then(serde_json::Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| match value {
                    serde_json::Value::String(text) => Some((key.clone(), text.clone())),
                    serde_json::Value::Bool(flag) => Some((key.clone(), flag.to_string())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}
