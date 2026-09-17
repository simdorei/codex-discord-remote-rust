//! Versioned evidence for one pre-delete refusal, not proof of a no-op sync.
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct CleanupRefusal {
    pub room: u64,
    pub reason: String,
}

impl CleanupRefusal {
    #[must_use]
    pub fn from_outcome(outcome: Option<&Value>) -> Option<Self> {
        let value = outcome?;
        if value["kind"] != "mirror_cleanup_refused"
            || value["version"] != 1
            || value["sync_completed"] != false
            || value["delete_dispatched"] != false
            || value["earlier_changes_possible"] != true
        {
            return None;
        }
        let room = value["blocked_room_id"]
            .as_u64()
            .filter(|id| *id > 0 && i64::try_from(*id).is_ok())?;
        let reason = value["protection_reason"].as_str()?;
        if !matches!(
            reason,
            "queued requests"
                | "prompt intake"
                | "ingress"
                | "undelivered result"
                | "undelivered progress"
                | "busy choice"
                | "undelivered goal progress"
                | "unattributable delivery receipt (invalid or missing channel identity)"
                | "unsettled delivery receipt (unknown, retryable or blocked)"
        ) {
            return None;
        }
        Some(Self {
            room,
            reason: reason.into(),
        })
    }

    #[must_use]
    pub fn outcome(&self) -> Value {
        json!({"kind":"mirror_cleanup_refused","version":1,"sync_completed":false,
            "blocked_room_id":self.room,"protection_reason":self.reason,
            "delete_dispatched":false,"earlier_changes_possible":true})
    }

    #[must_use]
    pub fn message(&self) -> String {
        format!(
            "Mirror sync stopped.\nroom: {}\nreason: {}\nNo deletion was dispatched for this room. Earlier sync changes may have completed.\nPending work is preserved; this request will not retry automatically.",
            self.room, self.reason
        )
    }

    #[must_use]
    pub fn pending_summary(&self) -> String {
        format!(
            "Mirror sync stopped: room {} protected by {}; notification confirmation not recorded. No deletion dispatched for this room; earlier changes may have completed. No automatic retry.",
            self.room, self.reason
        )
    }
}
