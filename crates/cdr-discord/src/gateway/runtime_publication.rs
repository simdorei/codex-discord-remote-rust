//! Synchronous publication contract for owned typed gateway ingress.
//!
//! Each decoded event updates sticky identity, moves into at most one typed lane with its original
//! receive timestamp, and then records the outcome. Receive failures use their own typed lane.
//!
//! Shutdown acceptance is cooperative: the ingress acceptance-flag read is the linearization
//! point. A publication already past that read may finish as in-flight work; shutdown is not a
//! retroactive hard barrier.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use tokio::time::Instant;
use twilight_model::gateway::event::Event;

use super::ingress::{GatewayIngress, PublishOutcome, ReceiveErrorPublishOutcome};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GatewayIngressPublishOutcomeSnapshot {
    pub ignored: u64,
    pub interactions_accepted: u64,
    pub hard_dropped_interactions: u64,
    pub messages_accepted: u64,
    pub recoverable_message_gaps: u64,
    pub receive_errors_accepted: u64,
    pub receive_errors_dropped: u64,
}

#[derive(Debug, Default)]
struct GatewayIngressPublishOutcomeState {
    ignored: AtomicU64,
    interactions_accepted: AtomicU64,
    hard_dropped_interactions: AtomicU64,
    messages_accepted: AtomicU64,
    recoverable_message_gaps: AtomicU64,
    receive_errors_accepted: AtomicU64,
    receive_errors_dropped: AtomicU64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct GatewayIngressPublishOutcomes {
    state: Arc<GatewayIngressPublishOutcomeState>,
}

impl GatewayIngressPublishOutcomes {
    pub(super) fn snapshot(&self) -> GatewayIngressPublishOutcomeSnapshot {
        GatewayIngressPublishOutcomeSnapshot {
            ignored: self.state.ignored.load(Ordering::Relaxed),
            interactions_accepted: self.state.interactions_accepted.load(Ordering::Relaxed),
            hard_dropped_interactions: self.state.hard_dropped_interactions.load(Ordering::Relaxed),
            messages_accepted: self.state.messages_accepted.load(Ordering::Relaxed),
            recoverable_message_gaps: self.state.recoverable_message_gaps.load(Ordering::Relaxed),
            receive_errors_accepted: self.state.receive_errors_accepted.load(Ordering::Relaxed),
            receive_errors_dropped: self.state.receive_errors_dropped.load(Ordering::Relaxed),
        }
    }

    fn record(&self, outcome: PublishOutcome) {
        let counter = match outcome {
            PublishOutcome::Ignored => &self.state.ignored,
            PublishOutcome::InteractionAccepted { .. } => &self.state.interactions_accepted,
            PublishOutcome::InteractionHardDropped { .. } => &self.state.hard_dropped_interactions,
            PublishOutcome::MessageAccepted { .. } => &self.state.messages_accepted,
            PublishOutcome::MessageRecoverableGap { .. } => &self.state.recoverable_message_gaps,
        };
        saturating_increment(counter);
    }

    fn record_receive_error(&self, outcome: ReceiveErrorPublishOutcome) {
        let counter = match outcome {
            ReceiveErrorPublishOutcome::Accepted => &self.state.receive_errors_accepted,
            ReceiveErrorPublishOutcome::Dropped { .. } => &self.state.receive_errors_dropped,
        };
        saturating_increment(counter);
    }
}

fn saturating_increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

pub(super) fn publish_decoded_event(
    event: Event,
    received_at: Instant,
    ingress: &GatewayIngress,
    outcomes: &GatewayIngressPublishOutcomes,
) {
    publish_decoded_event_with(
        event,
        received_at,
        ingress,
        || {},
        |outcome| {
            outcomes.record(outcome);
        },
    );
}

pub(super) fn publish_decoded_event_with(
    event: Event,
    received_at: Instant,
    ingress: &GatewayIngress,
    typed_observer: impl FnOnce(),
    outcome_observer: impl FnOnce(PublishOutcome),
) {
    let outcome = ingress.publish_with_observer(event, received_at, typed_observer);
    outcome_observer(outcome);
}

pub(super) fn publish_receive_error(
    shard: u32,
    message: String,
    ingress: &GatewayIngress,
    outcomes: &GatewayIngressPublishOutcomes,
) -> ReceiveErrorPublishOutcome {
    let outcome = ingress.publish_receive_error(shard, message);
    outcomes.record_receive_error(outcome);
    outcome
}
