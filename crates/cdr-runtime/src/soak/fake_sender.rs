use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::session_mirror_worker::{SessionMirrorDeliveryIdentity, SessionMirrorSender};

use super::success_tracker::{DiskSuccessTracker, MIRROR_NAMESPACE};

pub(super) struct FakeSessionMirrorSender {
    state: Mutex<SenderState>,
    tracker: Arc<DiskSuccessTracker>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SenderMetrics {
    pub attempts: u64,
    pub failures: u64,
    pub successes: u64,
    pub duplicate_successes: u64,
    pub matching_retries: u64,
}

#[derive(Default)]
struct SenderState {
    fail_next: bool,
    metrics: SenderMetrics,
    failed_delivery: Option<String>,
}

impl FakeSessionMirrorSender {
    pub fn new(tracker: Arc<DiskSuccessTracker>) -> Self {
        Self {
            state: Mutex::new(SenderState::default()),
            tracker,
        }
    }

    pub fn schedule_one_failure(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fail_next = true;
    }

    pub fn metrics(&self) -> SenderMetrics {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .metrics
    }
}

impl SessionMirrorSender for FakeSessionMirrorSender {
    fn send<'a>(
        &'a self,
        channel_id: u64,
        identity: &'a SessionMirrorDeliveryIdentity,
        _text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let delivery_key = format!(
                "{channel_id}:{}:{}",
                identity.domain(),
                identity.logical_key()
            );
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.metrics.attempts += 1;
            if state.fail_next {
                state.fail_next = false;
                state.metrics.failures += 1;
                state.failed_delivery = Some(delivery_key);
                return Err("deterministic offline send failure".into());
            }
            let record = self
                .tracker
                .record(MIRROR_NAMESPACE, &delivery_key)
                .map_err(|error| error.to_string())?;
            state.metrics.successes += 1;
            if state.failed_delivery.as_deref() == Some(&delivery_key) {
                state.metrics.matching_retries += 1;
                state.failed_delivery = None;
            }
            state.metrics.duplicate_successes = record.duplicate_count;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stable_delivery_identities_are_deduped_across_the_full_run() {
        let temp = tempfile::tempdir().unwrap();
        let tracker =
            Arc::new(DiskSuccessTracker::create(&temp.path().join("successes.sqlite")).unwrap());
        let sender = FakeSessionMirrorSender::new(tracker);
        let message_a = SessionMirrorDeliveryIdentity::event("thread-a", "event-a");
        let message_b = SessionMirrorDeliveryIdentity::event("thread-a", "event-b");
        sender.schedule_one_failure();
        assert!(sender.send(1, &message_a, "message-a").await.is_err());
        sender.send(1, &message_b, "message-b").await.unwrap();
        sender.send(1, &message_a, "message-a").await.unwrap();
        sender
            .send(1, &message_a, "changed retry text")
            .await
            .unwrap();
        sender.send(2, &message_a, "message-a").await.unwrap();

        let metrics = sender.metrics();
        assert_eq!(metrics.matching_retries, 1);
        assert_eq!(metrics.successes, 4);
        assert_eq!(metrics.duplicate_successes, 1);
    }
}
