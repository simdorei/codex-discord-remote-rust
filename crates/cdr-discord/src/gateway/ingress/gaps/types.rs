use std::sync::Arc;

use thiserror::Error;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use super::super::UnavailableReason;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MessageGapPosition {
    pub timestamp_micros: i64,
    pub message_id: Id<MessageMarker>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MessageGapReasons(u8);

impl MessageGapReasons {
    const FULL: u8 = 1 << 0;
    const CLOSED: u8 = 1 << 1;
    const STOPPING: u8 = 1 << 2;
    const SEQUENCE_EXHAUSTED: u8 = 1 << 3;

    pub(super) fn only(reason: UnavailableReason) -> Self {
        let mut reasons = Self::default();
        reasons.insert(reason);
        reasons
    }

    pub(super) fn insert(&mut self, reason: UnavailableReason) {
        self.0 |= match reason {
            UnavailableReason::Full => Self::FULL,
            UnavailableReason::Closed => Self::CLOSED,
            UnavailableReason::Stopping => Self::STOPPING,
            UnavailableReason::SequenceExhausted => Self::SEQUENCE_EXHAUSTED,
        };
    }

    #[must_use]
    pub fn contains(self, reason: UnavailableReason) -> bool {
        Self::only(reason).0 & self.0 != 0
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageGapSnapshot {
    pub channel_id: Id<ChannelMarker>,
    pub earliest: MessageGapPosition,
    pub reasons: MessageGapReasons,
    pub observation_count: u64,
    pub revision: u64,
}

/// Opaque recovery ownership token. It is intentionally not cloneable.
///
/// ```compile_fail
/// use cdr_discord::gateway::ingress::MessageGapNotice;
/// fn duplicate(notice: &MessageGapNotice) {
///     let _ = <MessageGapNotice as Clone>::clone(notice);
/// }
/// ```
///
/// ```compile_fail
/// use cdr_discord::gateway::ingress::MessageGapNotice;
/// fn rewrite_revision(mut notice: MessageGapNotice) {
///     notice.snapshot.revision = 99;
/// }
/// ```
#[derive(Debug)]
pub struct MessageGapNotice {
    pub(super) snapshot: MessageGapSnapshot,
    pub(super) tracker_identity: Arc<()>,
}

impl MessageGapNotice {
    #[must_use]
    pub fn snapshot(&self) -> MessageGapSnapshot {
        self.snapshot
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MessageGapStateError {
    #[error("Discord message gap state lock is poisoned")]
    Poisoned,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MessageGapAckError {
    #[error("Discord message gap notice belongs to another tracker")]
    ForeignTracker,
    #[error("Discord message gap revision is exhausted and cannot be acknowledged safely")]
    RevisionExhausted,
    #[error("Discord message gap state lock is poisoned")]
    StatePoisoned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageGapAckOutcome {
    Cleared,
    Stale,
}

#[cfg(test)]
mod tests {
    use super::{MessageGapReasons, UnavailableReason};

    #[test]
    fn all_unavailable_reasons_have_distinct_membership_bits() {
        let mut reasons = MessageGapReasons::default();
        let all = [
            UnavailableReason::Full,
            UnavailableReason::Closed,
            UnavailableReason::Stopping,
            UnavailableReason::SequenceExhausted,
        ];
        for reason in all {
            reasons.insert(reason);
        }
        assert_eq!(reasons.len(), 4);
        assert!(all.into_iter().all(|reason| reasons.contains(reason)));
    }
}
