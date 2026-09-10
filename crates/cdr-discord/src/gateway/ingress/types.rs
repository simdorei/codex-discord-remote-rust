use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use thiserror::Error;
use tokio::{
    sync::{broadcast, mpsc},
    time::Instant,
};
use twilight_model::gateway::payload::incoming::{InteractionCreate, MessageCreate};

pub const DEFAULT_INTERACTION_CAPACITY: usize = 64;
pub const DEFAULT_RESERVED_INTERACTION_CAPACITY: usize = 4;
pub const DEFAULT_MESSAGE_CAPACITY: usize = 1_024;
pub const DEFAULT_RECEIVE_ERROR_CAPACITY: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayIngressConfig {
    pub interaction_capacity: usize,
    pub reserved_interaction_capacity: usize,
    pub message_capacity: usize,
    pub receive_error_capacity: usize,
}

impl GatewayIngressConfig {
    pub(super) fn validate(self) -> Result<(), GatewayIngressConfigError> {
        for (capacity, lane) in [
            (self.interaction_capacity, IngressLane::NormalInteraction),
            (
                self.reserved_interaction_capacity,
                IngressLane::ReservedInteraction,
            ),
            (self.message_capacity, IngressLane::Message),
            (self.receive_error_capacity, IngressLane::ReceiveError),
        ] {
            if capacity == 0 {
                return Err(GatewayIngressConfigError::ZeroCapacity(lane));
            }
        }
        Ok(())
    }
}

impl Default for GatewayIngressConfig {
    fn default() -> Self {
        Self {
            interaction_capacity: DEFAULT_INTERACTION_CAPACITY,
            reserved_interaction_capacity: DEFAULT_RESERVED_INTERACTION_CAPACITY,
            message_capacity: DEFAULT_MESSAGE_CAPACITY,
            receive_error_capacity: DEFAULT_RECEIVE_ERROR_CAPACITY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressLane {
    NormalInteraction,
    ReservedInteraction,
    Message,
    ReceiveError,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum GatewayIngressConfigError {
    #[error("Discord gateway {0:?} ingress capacity must be greater than zero")]
    ZeroCapacity(IngressLane),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionIngressTag {
    Normal,
    Busy,
    Stopping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    Full,
    Closed,
    Stopping,
    SequenceExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishOutcome {
    Ignored,
    InteractionAccepted {
        sequence: u64,
        tag: InteractionIngressTag,
    },
    InteractionHardDropped {
        reason: UnavailableReason,
    },
    MessageAccepted {
        sequence: u64,
    },
    MessageRecoverableGap {
        reason: UnavailableReason,
        tracking: Result<(), super::MessageGapStateError>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct InteractionIngress {
    pub sequence: u64,
    pub received_at: Instant,
    pub tag: InteractionIngressTag,
    pub event: Box<InteractionCreate>,
}

/// Owned gateway message envelope. It is intentionally not cloneable.
///
/// ```compile_fail
/// use cdr_discord::gateway::ingress::MessageIngress;
/// fn duplicate(message: &MessageIngress) {
///     let _ = <MessageIngress as Clone>::clone(message);
/// }
/// ```
#[derive(Debug, PartialEq)]
pub struct MessageIngress {
    pub sequence: u64,
    pub event: Box<MessageCreate>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ReceiveErrorIngress {
    pub shard: u32,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveErrorPublishOutcome {
    Accepted,
    Dropped { reason: UnavailableReason },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IngressDiagnostics {
    pub hard_dropped_interactions: u64,
    pub recoverable_message_gaps: u64,
}

#[derive(Debug, Default)]
pub(super) struct IngressDiagnosticState {
    hard_dropped_interactions: AtomicU64,
    recoverable_message_gaps: AtomicU64,
}

impl IngressDiagnosticState {
    pub(super) fn record_hard_drop(&self) {
        saturating_increment(&self.hard_dropped_interactions);
    }

    pub(super) fn record_recoverable_gap(&self) {
        saturating_increment(&self.recoverable_message_gaps);
    }

    fn snapshot(&self) -> IngressDiagnostics {
        IngressDiagnostics {
            hard_dropped_interactions: self.hard_dropped_interactions.load(Ordering::Relaxed),
            recoverable_message_gaps: self.recoverable_message_gaps.load(Ordering::Relaxed),
        }
    }
}

fn saturating_increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

#[derive(Debug)]
pub struct IngressDiagnosticsReceiver {
    state: Arc<IngressDiagnosticState>,
    notifications: broadcast::Receiver<()>,
}

impl IngressDiagnosticsReceiver {
    pub(super) fn new(
        state: Arc<IngressDiagnosticState>,
        notifications: broadcast::Receiver<()>,
    ) -> Self {
        Self {
            state,
            notifications,
        }
    }

    /// Recover authoritative counters even after a lagged notification.
    #[must_use]
    pub fn snapshot(&self) -> IngressDiagnostics {
        self.state.snapshot()
    }

    pub async fn changed(&mut self) -> Result<IngressDiagnostics, broadcast::error::RecvError> {
        self.notifications.recv().await?;
        Ok(self.snapshot())
    }

    pub fn try_changed(&mut self) -> Result<IngressDiagnostics, broadcast::error::TryRecvError> {
        self.notifications.try_recv()?;
        Ok(self.snapshot())
    }
}

#[derive(Debug)]
pub struct GatewayIngressReceivers {
    pub normal_interactions: mpsc::Receiver<InteractionIngress>,
    pub reserved_interactions: mpsc::Receiver<InteractionIngress>,
    pub messages: mpsc::Receiver<MessageIngress>,
    pub receive_errors: mpsc::Receiver<ReceiveErrorIngress>,
}

pub(super) fn unavailable_reason<T>(error: &mpsc::error::TrySendError<T>) -> UnavailableReason {
    match error {
        mpsc::error::TrySendError::Full(_) => UnavailableReason::Full,
        mpsc::error::TrySendError::Closed(_) => UnavailableReason::Closed,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::saturating_increment;

    #[test]
    fn gi_in_14_atomic_diagnostic_counter_saturates() {
        let counter = AtomicU64::new(u64::MAX);
        saturating_increment(&counter);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}
