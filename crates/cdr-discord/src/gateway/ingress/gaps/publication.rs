use std::sync::MutexGuard;

use twilight_model::gateway::payload::incoming::MessageCreate;

use super::{
    ActiveGap, MessageGapPosition, MessageGapReasons, MessageGapStateError, MessageGapTracker,
};
use crate::gateway::ingress::UnavailableReason;

pub(in crate::gateway::ingress) struct MessageGapPublication<'a> {
    tracker: &'a MessageGapTracker,
    _gate: MutexGuard<'a, ()>,
}

impl MessageGapTracker {
    pub(in crate::gateway::ingress) fn begin_publication(&self) -> MessageGapPublication<'_> {
        let gate = self
            .publication_gate
            .lock()
            .expect("Discord message publication gate poisoned after a panic");
        MessageGapPublication {
            tracker: self,
            _gate: gate,
        }
    }
}

impl MessageGapPublication<'_> {
    pub(in crate::gateway::ingress) fn record(
        &self,
        message: &MessageCreate,
        reason: UnavailableReason,
    ) -> Result<(), MessageGapStateError> {
        let position = MessageGapPosition {
            timestamp_micros: message.timestamp.as_micros(),
            message_id: message.id,
        };
        let result = (|| {
            let mut entries = self
                .tracker
                .entries
                .lock()
                .map_err(|_| MessageGapStateError::Poisoned)?;
            let entry = entries.entry(message.channel_id).or_default();
            entry.revision = entry.revision.saturating_add(1);
            if let Some(active) = &mut entry.active {
                active.earliest = active.earliest.min(position);
                active.reasons.insert(reason);
                active.observation_count = active.observation_count.saturating_add(1);
            } else {
                entry.active = Some(ActiveGap {
                    earliest: position,
                    reasons: MessageGapReasons::only(reason),
                    observation_count: 1,
                });
            }
            Ok(())
        })();
        let _ = self.tracker.notifications.send(());
        result
    }
}
