//! The account of calls a task from another device makes to assistants.
//!
//! At full trust the task may call this device's other assistants and the
//! assistants shared with this device. Each call is the same device's work:
//! it takes a place under the share's concurrent tasks for as long as it
//! runs, is recorded in the audit log under that device, and a call to this
//! device's assistant runs in the workspace that device has under it, whose
//! session is kept and removed with the task's own.

use std::path::PathBuf;

use openbitfun_core::agentic::coordination::{
    DelegatedCall, DelegatedCallHold, DelegatedCallLedger, DelegatedCallTicket,
};
use openbitfun_core::service::delegation::DelegationTrust;
use openbitfun_core::service::delegation_audit::{DelegationAuditEvent, DelegationAuditLog};
use openbitfun_core::service::lan_share::{ensure_tenant, SharedAssistantStore, TenantUser};

use super::protocol::DispatchSubmitRequest;
use super::share_guard::{call_has_place, share_of, GRANT_WITHDRAWN};
use super::store::{remove_file_if_present, DispatchStore};

/// Why a call is refused when the owner allows no calls to other assistants.
const NORMAL_TRUST_REFUSAL: &str = "this share does not allow calling other assistants";

pub(super) struct DispatchCallLedger {
    store: DispatchStore,
    job_id: String,
    request: DispatchSubmitRequest,
    grants: SharedAssistantStore,
    tenants_root: PathBuf,
    audit: Option<DelegationAuditLog>,
    device_id: Option<String>,
}

impl DispatchCallLedger {
    pub(super) fn new(
        store: DispatchStore,
        job_id: String,
        request: DispatchSubmitRequest,
        grants: SharedAssistantStore,
        tenants_root: PathBuf,
        audit: Option<DelegationAuditLog>,
    ) -> Self {
        Self {
            store,
            job_id,
            request,
            grants,
            tenants_root,
            audit,
            device_id: None,
        }
    }

    /// Names this device in call chains by its sharing id.
    pub(super) fn with_device_id(mut self, device_id: Option<String>) -> Self {
        self.device_id = device_id;
        self
    }

    fn record(&self, call: &DelegatedCall) {
        let Some(log) = &self.audit else {
            return;
        };
        let Some(mut entry) = super::audit::entry(
            &self.request,
            DelegationAuditEvent::AssistantCalled,
            DelegationTrust::Full,
            Vec::new(),
            None,
            None,
        ) else {
            return;
        };
        entry.called_assistant = Some(call.agent_type.clone());
        if let Err(error) = log.append(&entry) {
            tracing::warn!(
                "Failed to record a call to another assistant in the audit log: {error}"
            );
        }
    }
}

#[async_trait::async_trait]
impl DelegatedCallLedger for DispatchCallLedger {
    fn local_device_id(&self) -> Option<String> {
        self.device_id.clone()
    }

    async fn open(&self, call: &DelegatedCall) -> Result<DelegatedCallTicket, String> {
        let Some((assistant_id, share_key)) = share_of(&self.request) else {
            return Err("this task did not come from another device".to_string());
        };
        // Access and trust are read now: the owner may have changed either
        // since the task started.
        let (share, grant) = match self.grants.authorize(assistant_id, share_key).await {
            Ok(Ok(granted)) => granted,
            Ok(Err(_)) => return Err(GRANT_WITHDRAWN.to_string()),
            Err(error) => return Err(format!("shared assistants cannot be read: {error}")),
        };
        if share.settings.trust != DelegationTrust::Full {
            return Err(NORMAL_TRUST_REFUSAL.to_string());
        }
        let tenant_workspace = match &call.local_assistant_id {
            Some(called) => Some(
                ensure_tenant(
                    &self.tenants_root,
                    called,
                    &grant.peer.device_id,
                    &TenantUser {
                        display_name: grant.peer.display_name.clone(),
                        device_name: grant.peer.device_name.clone(),
                    },
                )
                .map_err(|error| {
                    format!("the workspace of the requesting device cannot be prepared: {error}")
                })?,
            ),
            None => None,
        };
        let slot = {
            let place_error =
                |error: anyhow::Error| format!("the share's tasks cannot be counted: {error:#}");
            let _admission = self
                .store
                .lock_share_admission(assistant_id)
                .map_err(place_error)?;
            let max = share.settings.max_concurrent_tasks;
            if !call_has_place(&self.store, assistant_id, max).map_err(place_error)? {
                return Err(format!(
                    "this share already runs as many tasks as its owner allows ({max}); try again when one of them finishes"
                ));
            }
            self.store
                .add_call_slot(&self.job_id)
                .map_err(place_error)?
        };
        self.record(call);
        tracing::info!(
            "Task from another device opened a call: job_id={}, called={}",
            self.job_id,
            call.agent_type
        );
        Ok(DelegatedCallTicket::new(
            tenant_workspace,
            Box::new(CallHold {
                store: self.store.clone(),
                job_id: self.job_id.clone(),
                slot,
            }),
        ))
    }
}

/// One open call's place; dropping it gives the place back.
struct CallHold {
    store: DispatchStore,
    job_id: String,
    slot: PathBuf,
}

impl DelegatedCallHold for CallHold {
    fn record_session(&self, workspace_path: &str, session_id: &str) {
        if let Err(error) = self
            .store
            .record_call_session(&self.job_id, workspace_path, session_id)
        {
            tracing::warn!(
                "Failed to record the session of a call for retention: job_id={}, error={error:#}",
                self.job_id
            );
        }
    }
}

impl Drop for CallHold {
    fn drop(&mut self) {
        remove_file_if_present(&self.slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::delegation::test_support::{peer, revoke_key, share_assistant, share_key};
    use crate::dispatch::protocol::{
        DispatchApprovalPolicy, DispatchDelegation, DispatchDelegationSource, DispatchJobState,
        DispatchShareUser, DISPATCH_PROTOCOL_VERSION,
    };
    use openbitfun_core::service::delegation_audit::DelegationAuditEvent;
    use openbitfun_core::service::lan_share::{ShareConfirmations, ShareSettings};

    const SHARED: &str = "workspace";
    const CALLED: &str = "workspace-notes";

    fn request(job_id: &str, seed: u8) -> DispatchSubmitRequest {
        let user = peer(seed);
        DispatchSubmitRequest {
            protocol_version: DISPATCH_PROTOCOL_VERSION,
            job_id: job_id.to_string(),
            session_id: format!("session-{job_id}"),
            workspace_path: "/tmp".to_string(),
            agent_type: format!("assistant:{SHARED}"),
            prompt: "hello".to_string(),
            approval_policy: DispatchApprovalPolicy::Remote,
            model: None,
            reasoning_preset: None,
            title: None,
            attachments: Vec::new(),
            setup_audit: Vec::new(),
            delegation: Some(DispatchDelegation {
                assistant_id: SHARED.to_string(),
                source: DispatchDelegationSource::default(),
                call_chain: Vec::new(),
            }),
            source_share_key: Some(share_key(seed)),
            source_user: Some(DispatchShareUser {
                device_id: user.device_id.clone(),
                fingerprint: user.fingerprint.clone(),
                display_name: user.display_name.clone(),
                device_name: user.device_name.clone(),
            }),
            assistant_workspace: None,
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        store: DispatchStore,
        grants: SharedAssistantStore,
    }

    impl Fixture {
        /// `workspace` shared with device 1 at `trust`, two tasks at once.
        async fn new(trust: DelegationTrust) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = DispatchStore::open(dir.path().join("dispatch")).unwrap();
            let grants = SharedAssistantStore::new(dir.path().join("shared-assistants.json"));
            let settings = ShareSettings {
                trust,
                max_concurrent_tasks: 2,
                ..ShareSettings::default()
            };
            share_assistant(
                &grants,
                &dir.path().join("assistants"),
                SHARED,
                settings,
                &[1],
            )
            .await;
            Self { dir, store, grants }
        }

        fn running_job(&self, job_id: &str) {
            let job = request(job_id, 1);
            self.store
                .create_job_with_intent(job.clone(), job, job_id.to_string())
                .unwrap();
            self.store
                .mark_state(job_id, DispatchJobState::Running, None, None)
                .unwrap();
        }

        fn audit(&self) -> DelegationAuditLog {
            DelegationAuditLog::new(self.dir.path().join("audit.jsonl"))
        }

        fn ledger(&self, job_id: &str) -> DispatchCallLedger {
            DispatchCallLedger::new(
                self.store.clone(),
                job_id.to_string(),
                request(job_id, 1),
                self.grants.clone(),
                self.dir.path().join("tenants"),
                Some(self.audit()),
            )
        }
    }

    fn local_call() -> DelegatedCall {
        DelegatedCall {
            agent_type: format!("assistant:{CALLED}"),
            local_assistant_id: Some(CALLED.to_string()),
        }
    }

    #[tokio::test]
    async fn a_call_at_full_trust_runs_in_the_callers_workspace_under_the_called_assistant() {
        let fixture = Fixture::new(DelegationTrust::Full).await;
        fixture.running_job("delegation-1");
        let ticket = fixture
            .ledger("delegation-1")
            .open(&local_call())
            .await
            .expect("a call at full trust is admitted");

        let tenant = ticket
            .tenant_workspace
            .clone()
            .expect("a local call has a workspace");
        assert_eq!(
            tenant,
            fixture
                .dir
                .path()
                .join("tenants")
                .join(CALLED)
                .join(&peer(1).device_id)
        );
        assert!(tenant.join("USER.md").is_file());
        assert_eq!(fixture.store.call_slots("delegation-1"), 1);

        // The call is recorded under the device that delegated the task.
        let records = fixture.audit().read().unwrap();
        let entry = records[0].entry.as_ref().expect("a readable entry");
        assert_eq!(entry.event, DelegationAuditEvent::AssistantCalled);
        assert_eq!(
            entry.called_assistant.as_deref(),
            Some("assistant:workspace-notes")
        );
        assert_eq!(entry.assistant_id, SHARED);
        assert_eq!(entry.peer_display_name, "User 1");
        assert_eq!(entry.trust, "full");

        // Its place returns when the call ends.
        drop(ticket);
        assert_eq!(fixture.store.call_slots("delegation-1"), 0);

        // A call to an assistant shared with this device takes a place but
        // needs no workspace here.
        let shared = fixture
            .ledger("delegation-1")
            .open(&DelegatedCall {
                agent_type: "shared:laptop/writer".to_string(),
                local_assistant_id: None,
            })
            .await
            .unwrap();
        assert_eq!(shared.tenant_workspace, None);
        assert_eq!(fixture.store.call_slots("delegation-1"), 1);
    }

    #[tokio::test]
    async fn calls_count_against_the_share_and_are_refused_over_its_limit() {
        let fixture = Fixture::new(DelegationTrust::Full).await;
        fixture.running_job("delegation-1");
        let ledger = fixture.ledger("delegation-1");
        // The task itself and one call fill the share's two places.
        let first = ledger.open(&local_call()).await.unwrap();
        let refusal = ledger.open(&local_call()).await.unwrap_err();
        assert!(
            refusal.contains("as many tasks as its owner allows (2)"),
            "{refusal}"
        );
        assert_eq!(fixture.store.call_slots("delegation-1"), 1);

        // While the call runs, another task of the share waits for a place.
        let waiting = request("delegation-2", 1);
        fixture
            .store
            .create_job_with_intent(waiting.clone(), waiting, "delegation-2".to_string())
            .unwrap();
        assert!(!crate::dispatch::share_guard::call_has_place(&fixture.store, SHARED, 2).unwrap());
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                crate::dispatch::share_guard::wait_for_place(
                    &fixture.store,
                    &fixture.grants,
                    "delegation-2",
                ),
            )
            .await
            .is_err(),
            "a task started while a call held the last place"
        );

        drop(first);
        assert!(ledger.open(&local_call()).await.is_ok());
    }

    #[tokio::test]
    async fn calls_are_refused_at_normal_trust_or_once_access_ends() {
        let fixture = Fixture::new(DelegationTrust::Normal).await;
        fixture.running_job("delegation-1");
        let refusal = fixture
            .ledger("delegation-1")
            .open(&local_call())
            .await
            .unwrap_err();
        assert_eq!(refusal, NORMAL_TRUST_REFUSAL);

        // Raised to full trust, then the device's access is revoked.
        fixture
            .grants
            .change_settings(
                SHARED,
                ShareSettings {
                    trust: DelegationTrust::Full,
                    ..ShareSettings::default()
                },
                ShareConfirmations {
                    full_trust_risk: true,
                    external_agent_usage: false,
                },
            )
            .await
            .unwrap();
        assert!(fixture
            .ledger("delegation-1")
            .open(&local_call())
            .await
            .is_ok());
        revoke_key(&fixture.grants, SHARED, 1).await;
        let refusal = fixture
            .ledger("delegation-1")
            .open(&local_call())
            .await
            .unwrap_err();
        assert_eq!(refusal, GRANT_WITHDRAWN);
        assert!(fixture.audit().read().unwrap().len() == 1);
    }

    #[tokio::test]
    async fn a_calls_session_is_recorded_with_the_task() {
        let fixture = Fixture::new(DelegationTrust::Full).await;
        fixture.running_job("delegation-1");
        let ticket = fixture
            .ledger("delegation-1")
            .open(&local_call())
            .await
            .unwrap();
        ticket.record_session("/tenants/workspace-notes/device", "called-session");
        assert_eq!(
            fixture.store.call_sessions("delegation-1"),
            vec![(
                "/tenants/workspace-notes/device".to_string(),
                "called-session".to_string()
            )]
        );
    }
}
