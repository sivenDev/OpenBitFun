//! Host boundary for delegating subagent turns to assistants another device
//! shares with this one.
//!
//! The kernel addresses such an agent as `remote:<deviceId>/<openAgentId>`
//! and drives it as a detached dispatch job on that device: submit once, observe
//! the job's event log by cursor, answer its permission prompts, cancel it, or
//! continue it with a follow-up turn. Device discovery, transport, and the
//! dispatch wire stay in the implementing host.

use crate::PortResult;
use async_trait::async_trait;
use openbitfun_product_domains::tool_permissions::{PermissionReply, PermissionRequest};
use serde::{Deserialize, Serialize};

pub const REMOTE_AGENT_TYPE_PREFIX: &str = "remote:";
/// One open agent on one device.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentAddress {
    /// The id of the device that shares the agent.
    pub device_id: String,
    pub open_agent_id: String,
}

impl RemoteAgentAddress {
    /// The subagent type the kernel uses for this agent.
    pub fn agent_type(&self) -> String {
        format!(
            "{REMOTE_AGENT_TYPE_PREFIX}{}/{}",
            self.device_id, self.open_agent_id
        )
    }

    /// Parses `remote:<deviceId>/<openAgentId>`. Open agent ids never contain
    /// `/`, so the last separator splits the two parts.
    pub fn parse(agent_type: &str) -> Option<Self> {
        let (device_id, open_agent_id) = agent_type
            .strip_prefix(REMOTE_AGENT_TYPE_PREFIX)?
            .rsplit_once('/')?;
        let valid = |value: &str| {
            !value.is_empty()
                && !value
                    .chars()
                    .any(|ch| ch.is_whitespace() || ch.is_control())
        };
        (valid(device_id) && valid(open_agent_id)).then(|| Self {
            device_id: device_id.to_string(),
            open_agent_id: open_agent_id.to_string(),
        })
    }
}

/// Why a remote agent cannot take work right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAgentUnavailableReason {
    /// The device cannot be reached.
    Offline,
    /// The device runs an OpenBitFun version without agent delegation, or
    /// without a dispatch capability this host requires.
    Incompatible,
    /// The device is online but did not answer in time or refused the query.
    Unreachable,
    /// The device answered, but cannot run the assistant it opened right
    /// now, such as one that was removed.
    AgentUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RemoteAgentAvailability {
    Available,
    Unavailable {
        reason: RemoteAgentUnavailableReason,
        detail: String,
    },
}

impl RemoteAgentAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// How fresh a listing must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAgentListing {
    /// A recent cached answer is fine; used when building every turn's catalog.
    Cached,
    /// Query the devices again; used by management views.
    Refreshed,
}

/// One agent another device shares with this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentDescriptor {
    pub address: RemoteAgentAddress,
    pub device_name: String,
    /// The target-side agent the open agent runs, for example `Standard`.
    pub agent_type: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub availability: RemoteAgentAvailability,
}

/// The requesting session, recorded by the target for display and audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentSource {
    pub session_id: String,
    pub agent_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentSubmitRequest {
    pub address: RemoteAgentAddress,
    /// Caller-generated job id, so a retried submission cannot start two jobs.
    pub job_id: String,
    /// Caller-generated target session id.
    pub session_id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub source: RemoteAgentSource,
}

/// A submitted job. Persisted by the kernel so a restarted requester can
/// observe it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentJob {
    pub device_id: String,
    pub job_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAgentJobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl RemoteAgentJobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Why a queued job has not started on its device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentJobHold {
    pub reason: RemoteAgentHoldReason,
    /// The number of tasks the device's owner allows at once, for a
    /// connection limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteAgentHoldReason {
    /// The shared assistant already runs as many tasks for other devices as
    /// its owner allows.
    ShareLimit,
}

/// One page of a job's event log after `cursor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentJobSnapshot {
    pub state: RemoteAgentJobState,
    pub cursor: u64,
    /// Raw dispatch events. The kernel projects the ones it understands and
    /// skips the rest, so a newer target's events never fail observation.
    #[serde(default)]
    pub events: Vec<serde_json::Value>,
    #[serde(default)]
    pub pending_permissions: Vec<PermissionRequest>,
    /// The target no longer holds events before `cursor`; the page restarts
    /// from its oldest retained event.
    #[serde(default)]
    pub cursor_reset: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Why the job waits to start; older devices never send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<RemoteAgentJobHold>,
}

#[async_trait]
pub trait RemoteAgentDelegationPort: Send + Sync {
    /// Agents other devices of this account opened to this device, including
    /// ones that are currently unavailable.
    async fn list_agents(&self, listing: RemoteAgentListing) -> Vec<RemoteAgentDescriptor>;

    /// Submits a delegated job. Fails before anything runs when the device is
    /// offline, incompatible, or refuses the delegation.
    async fn submit(&self, request: RemoteAgentSubmitRequest) -> PortResult<RemoteAgentJob>;

    /// Reads the job's state and the events after `cursor`.
    async fn observe(
        &self,
        job: &RemoteAgentJob,
        cursor: u64,
    ) -> PortResult<RemoteAgentJobSnapshot>;

    /// Cancels the job. Returns `false` when it had already finished.
    async fn cancel(&self, job: &RemoteAgentJob) -> PortResult<bool>;

    /// Answers one of the job's pending permission prompts.
    async fn answer(
        &self,
        job: &RemoteAgentJob,
        request_id: &str,
        reply: PermissionReply,
    ) -> PortResult<()>;

    /// Starts the next turn of a finished job's session; `turn_id` makes a
    /// retried request idempotent.
    async fn continue_job(
        &self,
        job: &RemoteAgentJob,
        turn_id: &str,
        prompt: String,
    ) -> PortResult<()>;
}
