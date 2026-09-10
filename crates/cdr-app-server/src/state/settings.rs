use super::{RuntimeState, extract_thread_id};
use serde_json::Value;

impl RuntimeState {
    pub(crate) const fn notification_revision(&self) -> u64 {
        self.notification_revision
    }
    pub(crate) fn observed_thread_settings(&self, thread: &str) -> Option<(u64, Value)> {
        if self.closed_reason.is_some() {
            return None;
        }
        for (offset, notification) in self.notifications.iter().rev().enumerate() {
            if extract_thread_id(&notification.params).as_deref() != Some(thread) {
                continue;
            }
            if notification.method == "thread/closed"
                || (notification.method == "thread/status/changed"
                    && notification
                        .params
                        .pointer("/status/type")
                        .and_then(Value::as_str)
                        == Some("notLoaded"))
            {
                return None;
            }
            if notification.method == "thread/settings/updated" {
                return Some((
                    self.notification_revision.saturating_sub(offset as u64),
                    notification
                        .params
                        .get("threadSettings")
                        .cloned()
                        .unwrap_or(Value::Null),
                ));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(state: &mut RuntimeState, method: &str, params: Value) {
        state.record_notification(crate::Notification {
            method: method.into(),
            params,
        });
    }
    #[test]
    fn observation_is_thread_scoped_revisioned_and_invalidated_on_unload() {
        let mut state = RuntimeState::default();
        assert!(state.observed_thread_settings("a").is_none());
        event(
            &mut state,
            "thread/settings/updated",
            json!({"threadId":"a","threadSettings":{"model":"first"}}),
        );
        event(
            &mut state,
            "thread/settings/updated",
            json!({"threadId":"b","threadSettings":{"model":"other"}}),
        );
        assert_eq!(
            state.observed_thread_settings("a"),
            Some((1, json!({"model":"first"})))
        );
        event(
            &mut state,
            "thread/status/changed",
            json!({"threadId":"a","status":{"type":"notLoaded"}}),
        );
        assert!(state.observed_thread_settings("a").is_none());
        event(
            &mut state,
            "thread/settings/updated",
            json!({"threadId":"a","threadSettings":{"model":"second"}}),
        );
        assert_eq!(
            state.observed_thread_settings("a"),
            Some((4, json!({"model":"second"})))
        );
        state.closed_reason = Some("closed".into());
        assert!(state.observed_thread_settings("a").is_none());
    }
    #[test]
    fn old_observation_expires_with_the_bounded_notification_window() {
        let mut state = RuntimeState::default();
        event(
            &mut state,
            "thread/settings/updated",
            json!({"threadId":"a","threadSettings":{"model":"first"}}),
        );
        for _ in 0..super::super::MAX_NOTIFICATIONS {
            event(&mut state, "irrelevant", json!({}));
        }
        assert!(state.observed_thread_settings("a").is_none());
    }
}
