//! The owner's queue of access requests.
//!
//! Pure bookkeeping; every call takes the current time. A device has at most
//! one open request per assistant; the owner answers it within the request
//! window ([`REQUEST_SECONDS`] by default) or it is declined. After a
//! declined or expired request the device waits out a cooldown
//! ([`COOLDOWN_SECONDS`] by default) before asking for that assistant again. A request from a blocked device is declined at once,
//! without asking the owner.

use std::collections::HashMap;

use super::protocol::{Access, RequestState, RequestStatus, COOLDOWN_SECONDS, REQUEST_SECONDS};
use super::shared_assistants::SharePeer;

/// How long an answered request can still be looked up.
const OUTCOME_KEPT_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone)]
struct Outcome {
    public_key: String,
    granted: bool,
    at_ms: u64,
}

/// A request the owner has not answered yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub request_id: String,
    pub assistant_id: String,
    pub peer: SharePeer,
    pub opened_at_ms: u64,
    pub deadline_ms: u64,
}

/// What opening a request did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The owner is asked.
    Pending(PendingRequest),
    /// The device is blocked: the request is declined without asking.
    Declined { request_id: String },
}

/// Why a device cannot ask now.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestRefusal {
    #[error("this device already asked for this assistant; wait for the answer")]
    AlreadyPending {
        request_id: String,
        deadline_ms: u64,
    },
    #[error("this device can ask for this assistant again in {seconds} seconds")]
    Cooling { retry_at_ms: u64, seconds: u64 },
}

#[derive(Debug)]
pub struct RequestQueue {
    request_ms: u64,
    cooldown_ms: u64,
    pending: Vec<PendingRequest>,
    /// (public key, assistant) to the time the device may ask again.
    cooldowns: HashMap<(String, String), u64>,
    /// Answered requests: the asking key, whether they were granted, and
    /// when.
    outcomes: HashMap<String, Outcome>,
}

impl Default for RequestQueue {
    fn default() -> Self {
        Self::new(REQUEST_SECONDS * 1000, COOLDOWN_SECONDS * 1000)
    }
}

impl RequestQueue {
    /// A queue whose requests stay open `request_ms` and whose declined
    /// devices wait `cooldown_ms`.
    pub fn new(request_ms: u64, cooldown_ms: u64) -> Self {
        Self {
            request_ms,
            cooldown_ms,
            pending: Vec::new(),
            cooldowns: HashMap::new(),
            outcomes: HashMap::new(),
        }
    }

    /// Opens a request from `peer` for `assistant_id` at `now_ms`.
    pub fn open(
        &mut self,
        peer: SharePeer,
        assistant_id: &str,
        blocked: bool,
        now_ms: u64,
    ) -> Result<Opened, RequestRefusal> {
        self.forget_old(now_ms);
        let key = (peer.public_key.clone(), assistant_id.to_string());
        if let Some(open) = self
            .pending
            .iter()
            .find(|request| request.peer.public_key == key.0 && request.assistant_id == key.1)
        {
            return Err(RequestRefusal::AlreadyPending {
                request_id: open.request_id.clone(),
                deadline_ms: open.deadline_ms,
            });
        }
        if let Some(&retry_at_ms) = self.cooldowns.get(&key).filter(|&&until| until > now_ms) {
            return Err(RequestRefusal::Cooling {
                retry_at_ms,
                seconds: (retry_at_ms - now_ms).div_ceil(1000),
            });
        }
        let request_id = uuid::Uuid::new_v4().simple().to_string();
        if blocked {
            self.record(&request_id, &peer.public_key, false, now_ms);
            return Ok(Opened::Declined { request_id });
        }
        let request = PendingRequest {
            request_id,
            assistant_id: assistant_id.to_string(),
            peer,
            opened_at_ms: now_ms,
            deadline_ms: now_ms + self.request_ms,
        };
        self.pending.push(request.clone());
        Ok(Opened::Pending(request))
    }

    /// Requests the owner did not answer in time; each is declined and its
    /// device cools down.
    pub fn expire(&mut self, now_ms: u64) -> Vec<PendingRequest> {
        let (expired, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|request| request.deadline_ms <= now_ms);
        self.pending = open;
        for request in &expired {
            self.close(request, false, now_ms);
        }
        expired
    }

    /// Records the owner's answer to `request_id`. Returns the request, or
    /// none when it is no longer open.
    pub fn decide(
        &mut self,
        request_id: &str,
        granted: bool,
        now_ms: u64,
    ) -> Option<PendingRequest> {
        let index = self
            .pending
            .iter()
            .position(|request| request.request_id == request_id)?;
        let request = self.pending.remove(index);
        self.close(&request, granted, now_ms);
        Some(request)
    }

    /// Ends every open request for `assistant_id`, declined, such as when
    /// the owner stops sharing it. The devices do not cool down.
    pub fn withdraw_assistant(&mut self, assistant_id: &str, now_ms: u64) -> Vec<PendingRequest> {
        let (withdrawn, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|request| request.assistant_id == assistant_id);
        self.pending = open;
        for request in &withdrawn {
            self.record(&request.request_id, &request.peer.public_key, false, now_ms);
        }
        withdrawn
    }

    /// Where `request_id` of the device with `public_key` stands. Another
    /// device's request is unknown to it.
    pub fn status(&self, public_key: &str, request_id: &str) -> RequestStatus {
        if let Some(request) = self.pending.iter().find(|request| {
            request.request_id == request_id && request.peer.public_key == public_key
        }) {
            return RequestStatus {
                state: RequestState::Pending,
                deadline_ms: Some(request.deadline_ms),
            };
        }
        let state = match self
            .outcomes
            .get(request_id)
            .filter(|outcome| outcome.public_key == public_key)
        {
            Some(outcome) if outcome.granted => RequestState::Granted,
            Some(_) => RequestState::Declined,
            None => RequestState::Unknown,
        };
        RequestStatus {
            state,
            deadline_ms: None,
        }
    }

    /// The open request or cooldown of `public_key` for `assistant_id`.
    pub fn access(&self, public_key: &str, assistant_id: &str, now_ms: u64) -> Option<Access> {
        if let Some(request) = self.pending.iter().find(|request| {
            request.peer.public_key == public_key && request.assistant_id == assistant_id
        }) {
            return Some(Access::Pending {
                request_id: request.request_id.clone(),
                deadline_ms: request.deadline_ms,
            });
        }
        self.cooldowns
            .get(&(public_key.to_string(), assistant_id.to_string()))
            .filter(|&&until| until > now_ms)
            .map(|&retry_at_ms| Access::Cooling { retry_at_ms })
    }

    /// Open requests, oldest first.
    pub fn pending(&self) -> &[PendingRequest] {
        &self.pending
    }

    /// The open request with `request_id`.
    pub fn get(&self, request_id: &str) -> Option<&PendingRequest> {
        self.pending
            .iter()
            .find(|request| request.request_id == request_id)
    }

    fn record(&mut self, request_id: &str, public_key: &str, granted: bool, now_ms: u64) {
        self.outcomes.insert(
            request_id.to_string(),
            Outcome {
                public_key: public_key.to_string(),
                granted,
                at_ms: now_ms,
            },
        );
    }

    fn close(&mut self, request: &PendingRequest, granted: bool, now_ms: u64) {
        self.record(
            &request.request_id,
            &request.peer.public_key,
            granted,
            now_ms,
        );
        let key = (
            request.peer.public_key.clone(),
            request.assistant_id.clone(),
        );
        if granted {
            self.cooldowns.remove(&key);
        } else {
            self.cooldowns.insert(key, now_ms + self.cooldown_ms);
        }
    }

    fn forget_old(&mut self, now_ms: u64) {
        self.cooldowns.retain(|_, until| *until > now_ms);
        self.outcomes
            .retain(|_, outcome| now_ms.saturating_sub(outcome.at_ms) < OUTCOME_KEPT_MS);
    }
}
