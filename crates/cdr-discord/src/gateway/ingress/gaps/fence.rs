use std::sync::Arc;

use thiserror::Error;
use twilight_model::id::{Id, marker::ChannelMarker};

use super::{MessageGapReceiver, MessageGapTracker};

/// Opaque proof of one channel's clear gap state at a specific revision.
///
/// The token is intentionally not cloneable so callers cannot accidentally reuse it for
/// independent poll cycles.
#[derive(Debug)]
pub struct MessageGapFence {
    channel_id: Id<ChannelMarker>,
    revision: u64,
    tracker_identity: Arc<()>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MessageGapFenceError {
    #[error("Discord message gap fence belongs to another tracker")]
    ForeignTracker,
    #[error("Discord message gap advanced for channel {channel_id}")]
    Advanced { channel_id: Id<ChannelMarker> },
    #[error("Discord message gap state lock is poisoned")]
    StatePoisoned,
}

impl MessageGapTracker {
    fn capture_clear_fence(
        &self,
        channel_id: Id<ChannelMarker>,
    ) -> Result<Option<MessageGapFence>, super::MessageGapStateError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| super::MessageGapStateError::Poisoned)?;
        let entry = entries.get(&channel_id);
        if entry.is_some_and(|entry| entry.active.is_some()) {
            return Ok(None);
        }
        Ok(Some(MessageGapFence {
            channel_id,
            revision: entry.map_or(0, |entry| entry.revision),
            tracker_identity: Arc::clone(&self.identity),
        }))
    }

    fn with_current_fence<T>(
        &self,
        fence: &MessageGapFence,
        action: impl FnOnce() -> T,
    ) -> Result<T, MessageGapFenceError> {
        let _publication = self.begin_publication();
        if !Arc::ptr_eq(&self.identity, &fence.tracker_identity) {
            return Err(MessageGapFenceError::ForeignTracker);
        }
        let entries = self
            .entries
            .lock()
            .map_err(|_| MessageGapFenceError::StatePoisoned)?;
        let entry = entries.get(&fence.channel_id);
        let revision = entry.map_or(0, |entry| entry.revision);
        let is_clear = entry.is_none_or(|entry| entry.active.is_none());
        if revision != fence.revision || !is_clear {
            return Err(MessageGapFenceError::Advanced {
                channel_id: fence.channel_id,
            });
        }
        Ok(action())
    }
}

impl MessageGapReceiver {
    /// Capture a revision-bound token only while this channel has no pending gap.
    pub fn capture_clear_fence(
        &self,
        channel_id: Id<ChannelMarker>,
    ) -> Result<Option<MessageGapFence>, super::MessageGapStateError> {
        self.tracker.capture_clear_fence(channel_id)
    }

    /// Run a short synchronous action only if no gap appeared since the fence was captured.
    ///
    /// Validation and the action share the tracker lock, closing the check-to-action race. The
    /// action must not block, await, or call back into message-gap APIs.
    pub fn with_current_fence<T>(
        &self,
        fence: &MessageGapFence,
        action: impl FnOnce() -> T,
    ) -> Result<T, MessageGapFenceError> {
        self.tracker.with_current_fence(fence, action)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    use twilight_model::gateway::payload::incoming::MessageCreate;

    use super::*;
    use crate::gateway::ingress::UnavailableReason;

    fn message() -> MessageCreate {
        serde_json::from_value(serde_json::json!({
            "attachments": [], "author": {"avatar": null, "bot": false,
                "discriminator": "0001", "id": "3", "username": "tester"},
            "channel_id": "7", "content": "hello", "edited_timestamp": null,
            "embeds": [], "id": "42", "mention_everyone": false,
            "mention_roles": [], "mentions": [], "pinned": false,
            "timestamp": "2020-02-02T02:02:02.020000+00:00", "tts": false, "type": 0
        }))
        .expect("valid message")
    }

    #[test]
    fn gi_gap_fence_03_failure_decision_and_gap_record_are_atomic_against_discard() {
        let tracker = MessageGapTracker::new();
        let receiver = tracker.subscribe();
        let channel_id = Id::<ChannelMarker>::new(7);
        let fence = receiver
            .capture_clear_fence(channel_id)
            .unwrap()
            .expect("clean fence");
        let (failure_decided_tx, failure_decided_rx) = mpsc::channel();
        let (finish_record_tx, finish_record_rx) = mpsc::channel();
        let publisher = tracker.clone();
        let publisher = std::thread::spawn(move || {
            let publication = publisher.begin_publication();
            failure_decided_tx.send(()).unwrap();
            finish_record_rx.recv().unwrap();
            publication.record(&message(), UnavailableReason::Closed)
        });
        failure_decided_rx.recv().unwrap();

        let claim_ran = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&claim_ran);
        let claimant = std::thread::spawn(move || {
            receiver.with_current_fence(&fence, || observed.store(true, Ordering::SeqCst))
        });
        finish_record_tx.send(()).unwrap();

        assert_eq!(publisher.join().unwrap(), Ok(()));
        assert_eq!(
            claimant.join().unwrap(),
            Err(MessageGapFenceError::Advanced { channel_id })
        );
        assert!(!claim_ran.load(Ordering::SeqCst));
    }
}
