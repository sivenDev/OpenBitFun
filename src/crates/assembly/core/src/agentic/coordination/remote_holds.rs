//! Why a delegated subagent on another device makes no progress, kept so
//! its parent hears it while it waits instead of waiting in silence. Holds
//! live in memory by child session; a restarted host learns them again from
//! its next observation of the job.

use dashmap::DashMap;
use tokio::sync::futures::Notified;
use tokio::sync::Notify;

#[derive(Debug, Clone)]
struct Hold {
    note: String,
    /// Whether a wait already ended early to report this hold.
    reported: bool,
}

#[derive(Debug, Default)]
pub(crate) struct RemoteHolds {
    holds: DashMap<String, Hold>,
    changed: Notify,
}

impl RemoteHolds {
    /// Records why `session_id` waits. The same note again changes nothing,
    /// so it is reported once.
    pub(crate) fn set(&self, session_id: &str, note: String) {
        if self
            .holds
            .get(session_id)
            .is_some_and(|hold| hold.note == note)
        {
            return;
        }
        self.holds.insert(
            session_id.to_string(),
            Hold {
                note,
                reported: false,
            },
        );
        self.changed.notify_waiters();
    }

    pub(crate) fn clear(&self, session_id: &str) {
        if self.holds.remove(session_id).is_some() {
            self.changed.notify_waiters();
        }
    }

    /// Why `session_id` waits now, if it does.
    pub(crate) fn note(&self, session_id: &str) -> Option<String> {
        self.holds.get(session_id).map(|hold| hold.note.clone())
    }

    /// Whether `session_id` waits for a reason no wait has reported yet.
    pub(crate) fn unreported(&self, session_id: &str) -> bool {
        self.holds
            .get(session_id)
            .is_some_and(|hold| !hold.reported)
    }

    pub(crate) fn mark_reported(&self, session_id: &str) {
        if let Some(mut hold) = self.holds.get_mut(session_id) {
            hold.reported = true;
        }
    }

    /// Resolves at the next change of any hold.
    pub(crate) fn changed(&self) -> Notified<'_> {
        self.changed.notified()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_is_reported_once_until_it_changes() {
        let holds = RemoteHolds::default();
        holds.set("child", "Waiting for the owner".to_string());
        assert!(holds.unreported("child"));
        holds.mark_reported("child");
        assert!(!holds.unreported("child"));
        // The same reason again is the same hold.
        holds.set("child", "Waiting for the owner".to_string());
        assert!(!holds.unreported("child"));
        // A new reason is reported again.
        holds.set("child", "Queued".to_string());
        assert!(holds.unreported("child"));
        assert_eq!(holds.note("child").as_deref(), Some("Queued"));
        holds.clear("child");
        assert_eq!(holds.note("child"), None);
        assert!(!holds.unreported("child"));
    }
}
