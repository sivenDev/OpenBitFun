#![cfg(feature = "remote-agent-port")]

use async_trait::async_trait;
use openbitfun_product_domains::tool_permissions::PermissionReply;
use openbitfun_runtime_ports::{
    remote_connection_target, remote_target_connection, PortError, PortErrorKind, PortResult,
    RemoteAgentAddress, RemoteAgentAvailability, RemoteAgentDelegationPort, RemoteAgentDescriptor,
    RemoteAgentJob, RemoteAgentJobSnapshot, RemoteAgentJobState, RemoteAgentListing,
    RemoteAgentSource, RemoteAgentSubmitRequest, RemoteAgentUnavailableReason,
};
use serde_json::json;
use std::sync::Mutex;

#[test]
fn agent_types_round_trip_and_reject_malformed_addresses() {
    let address = RemoteAgentAddress {
        device_id: "device-a".to_string(),
        open_agent_id: "helper".to_string(),
    };
    assert_eq!(address.agent_type(), "remote:device-a/helper");
    assert_eq!(
        RemoteAgentAddress::parse("remote:device-a/helper"),
        Some(address)
    );
    // Device ids may contain `/`; open agent ids never do.
    assert_eq!(
        RemoteAgentAddress::parse("remote:org/device/helper").map(|a| a.device_id),
        Some("org/device".to_string())
    );
    for malformed in [
        "remote:",
        "remote:device-a",
        "remote:/helper",
        "remote:device-a/",
        "remote:device a/helper",
        "acp:claude-code",
        "Standard",
    ] {
        assert_eq!(RemoteAgentAddress::parse(malformed), None, "{malformed}");
    }
}

#[test]
fn availability_and_descriptors_use_stable_wire_names() {
    let descriptor = RemoteAgentDescriptor {
        address: RemoteAgentAddress {
            device_id: "laptop".to_string(),
            open_agent_id: "helper".to_string(),
        },
        device_name: "Laptop".to_string(),
        agent_type: "assistant:workspace".to_string(),
        display_name: "Helper".to_string(),
        description: None,
        availability: RemoteAgentAvailability::Unavailable {
            reason: RemoteAgentUnavailableReason::Offline,
            detail: "Laptop is offline".to_string(),
        },
        account: None,
    };
    let value = serde_json::to_value(&descriptor).unwrap();
    assert_eq!(value["address"]["openAgentId"], "helper");
    assert!(value.get("taskKinds").is_none());
    assert_eq!(value["availability"]["state"], "unavailable");
    assert_eq!(value["availability"]["reason"], "offline");
    assert!(!descriptor.availability.is_available());
    assert_eq!(
        serde_json::from_value::<RemoteAgentDescriptor>(value).unwrap(),
        descriptor
    );
}

#[test]
fn connection_agents_name_their_account_and_older_descriptors_have_none() {
    let address = RemoteAgentAddress {
        device_id: remote_connection_target("c0ffee"),
        open_agent_id: "helper".to_string(),
    };
    assert_eq!(address.agent_type(), "remote:conn:c0ffee/helper");
    assert_eq!(
        RemoteAgentAddress::parse("remote:conn:c0ffee/helper").as_ref(),
        Some(&address)
    );
    assert_eq!(remote_target_connection(&address.device_id), Some("c0ffee"));
    assert_eq!(remote_target_connection("laptop"), None);
    assert_eq!(remote_target_connection("conn:"), None);

    // A descriptor written before accounts were added still reads.
    let legacy: RemoteAgentDescriptor = serde_json::from_value(json!({
        "address": { "deviceId": "laptop", "openAgentId": "helper" },
        "deviceName": "Laptop",
        "agentType": "Standard",
        "displayName": "Helper",
        "taskKinds": ["message"],
        "availability": { "state": "available" },
    }))
    .unwrap();
    assert_eq!(legacy.account, None);
    let value = serde_json::to_value(&legacy).unwrap();
    assert!(value.get("account").is_none());
}

#[test]
fn snapshots_read_target_status_payloads_and_newer_states() {
    // A target status response carries fields this port does not model.
    let snapshot: RemoteAgentJobSnapshot = serde_json::from_value(json!({
        "state": "running",
        "cursor": 7,
        "events": [{"type": "jobState", "state": "running"}],
        "pendingPermissions": [],
        "cursorReset": false,
        "historyTruncated": false,
        "eventLogComplete": true,
        "omittedEventCount": 0
    }))
    .unwrap();
    assert_eq!(snapshot.state, RemoteAgentJobState::Running);
    assert_eq!(snapshot.cursor, 7);
    assert_eq!(snapshot.events.len(), 1);

    let newer: RemoteAgentJobSnapshot =
        serde_json::from_value(json!({"state": "paused", "cursor": 0})).unwrap();
    assert_eq!(newer.state, RemoteAgentJobState::Unknown);
    assert!(!newer.state.is_terminal());
    assert!(RemoteAgentJobState::Cancelled.is_terminal());
}

/// Fake host: one device that runs a message job to completion on the
/// second observation unless it is cancelled first.
#[derive(Default)]
struct FakeHost {
    observations: Mutex<u64>,
    cancelled: Mutex<bool>,
    answers: Mutex<Vec<(String, PermissionReply)>>,
}

#[async_trait]
impl RemoteAgentDelegationPort for FakeHost {
    async fn list_agents(&self, _listing: RemoteAgentListing) -> Vec<RemoteAgentDescriptor> {
        Vec::new()
    }

    async fn submit(&self, request: RemoteAgentSubmitRequest) -> PortResult<RemoteAgentJob> {
        if request.address.device_id != "laptop" {
            return Err(PortError::new(
                PortErrorKind::NotAvailable,
                "device is offline; nothing ran on this device instead",
            ));
        }
        Ok(RemoteAgentJob {
            device_id: request.address.device_id,
            job_id: request.job_id,
            session_id: request.session_id,
        })
    }

    async fn observe(
        &self,
        _job: &RemoteAgentJob,
        cursor: u64,
    ) -> PortResult<RemoteAgentJobSnapshot> {
        let mut observations = self.observations.lock().unwrap();
        *observations += 1;
        let state = if *self.cancelled.lock().unwrap() {
            RemoteAgentJobState::Cancelled
        } else if *observations >= 2 {
            RemoteAgentJobState::Succeeded
        } else {
            RemoteAgentJobState::Running
        };
        Ok(RemoteAgentJobSnapshot {
            state,
            cursor: cursor + 1,
            events: Vec::new(),
            pending_permissions: Vec::new(),
            cursor_reset: false,
            last_error: None,
        })
    }

    async fn cancel(&self, _job: &RemoteAgentJob) -> PortResult<bool> {
        let mut cancelled = self.cancelled.lock().unwrap();
        let was_running = !*cancelled;
        *cancelled = true;
        Ok(was_running)
    }

    async fn answer(
        &self,
        _job: &RemoteAgentJob,
        request_id: &str,
        reply: PermissionReply,
    ) -> PortResult<()> {
        self.answers
            .lock()
            .unwrap()
            .push((request_id.to_string(), reply));
        Ok(())
    }

    async fn continue_job(
        &self,
        _job: &RemoteAgentJob,
        _turn_id: &str,
        _prompt: String,
    ) -> PortResult<()> {
        Ok(())
    }
}

fn submit_request(device_id: &str) -> RemoteAgentSubmitRequest {
    RemoteAgentSubmitRequest {
        address: RemoteAgentAddress {
            device_id: device_id.to_string(),
            open_agent_id: "helper".to_string(),
        },
        job_id: "job-1".to_string(),
        session_id: "session-1".to_string(),
        prompt: "Summarize the notes".to_string(),
        title: None,
        source: RemoteAgentSource {
            session_id: "parent".to_string(),
            agent_type: "Standard".to_string(),
            display_name: None,
        },
    }
}

#[tokio::test]
async fn jobs_are_observed_by_cursor_until_terminal() {
    let host = FakeHost::default();
    let port: &dyn RemoteAgentDelegationPort = &host;
    let job = port.submit(submit_request("laptop")).await.unwrap();
    let mut cursor = 0;
    let state = loop {
        let snapshot = port.observe(&job, cursor).await.unwrap();
        assert!(snapshot.cursor > cursor, "cursor advances");
        cursor = snapshot.cursor;
        if snapshot.state.is_terminal() {
            break snapshot.state;
        }
    };
    assert_eq!(state, RemoteAgentJobState::Succeeded);

    port.answer(&job, "request-1", PermissionReply::Once)
        .await
        .unwrap();
    assert_eq!(host.answers.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancellation_and_offline_devices_are_explicit() {
    let host = FakeHost::default();
    let port: &dyn RemoteAgentDelegationPort = &host;
    let error = port.submit(submit_request("phone")).await.unwrap_err();
    assert_eq!(error.kind, PortErrorKind::NotAvailable);

    let job = port.submit(submit_request("laptop")).await.unwrap();
    assert!(port.cancel(&job).await.unwrap());
    assert!(!port.cancel(&job).await.unwrap());
    assert_eq!(
        port.observe(&job, 0).await.unwrap().state,
        RemoteAgentJobState::Cancelled
    );
}
