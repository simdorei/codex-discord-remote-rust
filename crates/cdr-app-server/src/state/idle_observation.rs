use super::RuntimeState;
use crate::Notification;

impl RuntimeState {
    /// Match the next exact queued notification, never the current/latest revision.
    pub(crate) fn confirm_idle_observation(&mut self, notification: &Notification) -> bool {
        let first = self
            .notification_revision
            .saturating_sub(self.notifications.len() as u64)
            + 1;
        let next = self.idle_observed_revision + 1;
        if next < first {
            self.idle_observation_gap = true;
            return false;
        }
        let index = usize::try_from(next - first).unwrap_or(usize::MAX);
        if self.notifications.get(index) != Some(notification) {
            self.idle_observation_gap = true;
            return false;
        }
        self.idle_observed_revision = next;
        true
    }

    pub(crate) fn idle_observations_caught_up(&self) -> bool {
        !self.idle_observation_gap && self.idle_observed_revision == self.notification_revision
    }

    pub(crate) fn witnessed_idle_terminal(&self, thread: &str, turn: &str) -> bool {
        self.notifications
            .iter()
            .rev()
            .find(|n| {
                matches!(n.method.as_str(), "turn/started" | "turn/completed")
                    && super::extract_thread_id(&n.params).as_deref() == Some(thread)
            })
            .is_some_and(|n| {
                n.method == "turn/completed"
                    && super::extract_turn_id(&n.params).as_deref() == Some(turn)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn terminal(id: &str) -> Notification {
        Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"A","turn":{"id":id}}),
        }
    }
    #[test]
    fn ir3_ir4_only_ordered_exact_observation_and_current_terminal_can_prove_idle() {
        let mut state = RuntimeState::default();
        let one = terminal("T1");
        let two = terminal("T2");
        state.record_notification(one.clone());
        state.record_notification(two.clone());
        assert!(!state.idle_observations_caught_up());
        assert!(state.confirm_idle_observation(&one));
        assert!(!state.idle_observations_caught_up());
        assert!(state.confirm_idle_observation(&two));
        assert!(state.idle_observations_caught_up());
        assert!(!state.witnessed_idle_terminal("A", "T1"));
        assert!(state.witnessed_idle_terminal("A", "T2"));
    }
    #[test]
    fn ir3_skipped_observation_is_not_acknowledged_as_the_latest_revision() {
        let mut state = RuntimeState::default();
        let one = terminal("T1");
        let two = terminal("T2");
        state.record_notification(one.clone());
        state.record_notification(two.clone());
        assert!(!state.confirm_idle_observation(&two));
        assert!(state.confirm_idle_observation(&one));
        assert!(state.confirm_idle_observation(&two));
        assert!(
            !state.idle_observations_caught_up(),
            "unreconciled gap stays negative"
        );
    }
}
