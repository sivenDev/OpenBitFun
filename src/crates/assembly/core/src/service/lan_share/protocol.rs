//! What sharing devices say to each other over the LAN channel.
//!
//! A device that shares assistants answers `catalog`, `request`,
//! `request_status`, `remove` and `dispatch`, and pushes `decision`,
//! `revoked` and `job_updated`. A device it has not granted anything may
//! only read the catalog and ask (RFC F002 D3).

use serde::{Deserialize, Serialize};

/// The capability both sides offer for assistant sharing.
pub const ASSISTANT_SHARE_CAPABILITY: &str = "assistant_share_v1";

pub const METHOD_CATALOG: &str = "catalog";
pub const METHOD_REQUEST: &str = "request";
pub const METHOD_REQUEST_STATUS: &str = "request_status";
pub const METHOD_REMOVE: &str = "remove";
/// A Dispatch target command for a shared assistant; granted devices only.
pub const METHOD_DISPATCH: &str = "dispatch";

/// Pushed to the asking device when the owner decides, or the request ends.
pub const PUSH_DECISION: &str = "decision";
/// Pushed to a device whose access the owner withdrew.
pub const PUSH_REVOKED: &str = "revoked";
/// Pushed to the device that started a job when the job has news. A hint to
/// read the job now; it carries no content.
pub const PUSH_JOB_UPDATED: &str = "job_updated";

/// How long the owner has to answer a request.
pub const REQUEST_SECONDS: u64 = 60;
/// How long a device waits after a request was declined or ran out.
pub const COOLDOWN_SECONDS: u64 = 60;

/// One shared assistant as a device sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogAssistant {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub access: Access,
}

/// Where the asking device stands with one shared assistant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Access {
    /// It may ask.
    Available,
    /// It asked; the owner answers by `deadline_ms`.
    #[serde(rename_all = "camelCase")]
    Pending {
        request_id: String,
        deadline_ms: u64,
    },
    /// It may use the assistant.
    Granted,
    /// Its last request was declined or ran out; it may ask again then.
    #[serde(rename_all = "camelCase")]
    Cooling { retry_at_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResponse {
    pub assistants: Vec<CatalogAssistant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantBody {
    pub assistant_id: String,
}

/// The answer to `request`: the request is open until `deadline_ms`, or
/// already decided (`granted`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestOpened {
    pub request_id: String,
    pub deadline_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestStatusBody {
    pub request_id: String,
}

/// Where a request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestState {
    Pending,
    Granted,
    Declined,
    /// This device knows no such request, such as after it restarted.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestStatus {
    pub state: RequestState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
}

/// The `decision` push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub request_id: String,
    pub assistant_id: String,
    pub granted: bool,
    /// The assistant as the owner shares it, when granted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<CatalogAssistant>,
}

/// The `revoked` push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revoked {
    pub assistant_id: String,
}

/// A Dispatch target command, such as `dispatch_target_submit`, and its
/// arguments. The owner binds the arguments to the calling device's key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchBody {
    pub command: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobUpdated {
    pub job_id: String,
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}
