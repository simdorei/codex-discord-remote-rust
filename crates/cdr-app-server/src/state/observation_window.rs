use super::RuntimeState;
use crate::{
    AppServerError,
    observation::{
        OBSERVATION_PAGE_BYTES, OBSERVATION_PAGE_SIZE, ObservationWindow, SourceObservation,
    },
};
use std::io::{self, Write};

struct Budget {
    used: usize,
    limit: usize,
}
impl Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self
            .used
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("observation size overflow"))?;
        if next > self.limit {
            return Err(io::Error::other("observation payload budget"));
        }
        self.used = next;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl RuntimeState {
    pub(crate) fn observation_window(
        &self,
        after: u64,
        upper: Option<u64>,
    ) -> Result<ObservationWindow, AppServerError> {
        if self.notification_sequence_exhausted {
            return Err(crate::idle_release::held("source sequence exhausted"));
        }
        let upper = upper.unwrap_or(self.notification_revision);
        if upper > self.notification_revision || after > upper {
            return Err(crate::idle_release::held("invalid observation window"));
        }
        let first = self.notification_revision - self.notifications.len() as u64 + 1;
        let begin = after.saturating_add(1).max(first);
        let mut page = ObservationWindow {
            owner_id: String::new(),
            generation: 0,
            first_available: first,
            source_upper: self.notification_revision,
            upper,
            scanned_through: upper.min(begin - 1),
            events: Vec::new(),
        };
        let mut used = 0;
        for (offset, notification) in self.notifications.iter().enumerate() {
            let sequence = first + offset as u64;
            if sequence < begin {
                continue;
            }
            if sequence > upper || page.events.len() >= OBSERVATION_PAGE_SIZE {
                break;
            }
            let mut budget = Budget {
                used: used + notification.method.len(),
                limit: OBSERVATION_PAGE_BYTES,
            };
            let fits = serde_json::to_writer(&mut budget, &notification.params).is_ok();
            let notification = if fits {
                used = budget.used;
                Some(notification.clone())
            } else {
                None
            };
            page.events.push(SourceObservation {
                sequence,
                notification,
            });
            page.scanned_through = sequence;
        }
        Ok(page)
    }
    pub(crate) fn certify_observation_prefix(&mut self, through: u64) -> bool {
        if self.notification_sequence_exhausted || through > self.notification_revision {
            return false;
        }
        self.idle_ledger_revision = Some(through);
        self.idle_observation_gap = false;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event() -> crate::Notification {
        crate::Notification {
            method: "fixture/no-effect".into(),
            params: json!({"threadId":"A"}),
        }
    }
    #[test]
    fn identical_payload_occurrences_keep_original_sequence_and_fixed_upper() {
        let mut s = RuntimeState::default();
        s.record_notification(event());
        s.record_notification(event());
        let page = s.observation_window(0, Some(2)).unwrap();
        for _ in 0..100 {
            s.record_notification(event());
        }
        let again = s.observation_window(0, Some(page.upper)).unwrap();
        assert_eq!(
            again.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(again.upper, 2);
        assert_eq!(again.source_upper, 102);
    }
    #[test]
    fn evicted_prefix_and_oversize_position_are_not_silently_certified() {
        let mut s = RuntimeState::default();
        for _ in 0..1004 {
            s.record_notification(event());
        }
        let page = s.observation_window(0, Some(1004)).unwrap();
        assert_eq!(page.first_available, 5);
        assert_eq!(page.events.first().unwrap().sequence, 5);
        assert_eq!(page.events.len(), 32);
        s.record_notification(crate::Notification {
            method: "item/completed".into(),
            params: json!({"x":"x".repeat(OBSERVATION_PAGE_BYTES+1)}),
        });
        let page = s.observation_window(1004, None).unwrap();
        assert_eq!(page.events[0].sequence, 1005);
        assert!(page.events[0].notification.is_none());
    }
    #[test]
    fn sequence_exhaustion_and_legacy_ack_do_not_authorize_a_new_ledger_prefix() {
        let mut s = RuntimeState {
            idle_ledger_required: true,
            ..RuntimeState::default()
        };
        let n = event();
        s.record_notification(n.clone());
        assert!(s.confirm_idle_observation(&n));
        assert!(!s.idle_observations_caught_up());
        assert!(s.certify_observation_prefix(1));
        assert!(s.idle_observations_caught_up());
        s.notification_revision = u64::MAX;
        s.record_notification(event());
        assert!(s.observation_window(0, None).is_err());
        assert!(!s.certify_observation_prefix(u64::MAX));
    }
}
