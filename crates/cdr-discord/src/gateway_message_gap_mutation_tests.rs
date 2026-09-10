use tokio::time::Instant;
use twilight_model::id::{Id, marker::ChannelMarker};

use super::{
    ingress::{
        GatewayIngress, InteractionIngressTag, MessageGapAckError, MessageGapFenceError,
        PublishOutcome, UnavailableReason,
    },
    ingress_tests::{config, interaction_event, message_event_at},
};

#[test]
fn gi_gap_15_mixed_interaction_sequence_does_not_fabricate_a_message_gap() {
    let (ingress, mut receivers) = GatewayIngress::new_for_test(config(2, 1, 2), 100).unwrap();
    assert_eq!(
        ingress.publish(
            message_event_at(1_300, 480, "2026-01-01T00:00:01+00:00"),
            Instant::now()
        ),
        PublishOutcome::MessageAccepted { sequence: 101 }
    );
    assert_eq!(
        ingress.publish(interaction_event(1_301), Instant::now()),
        PublishOutcome::InteractionAccepted {
            sequence: 102,
            tag: InteractionIngressTag::Normal
        }
    );
    assert_eq!(
        ingress.publish(
            message_event_at(1_302, 480, "2026-01-01T00:00:02+00:00"),
            Instant::now()
        ),
        PublishOutcome::MessageAccepted { sequence: 103 }
    );
    assert!(
        ingress
            .subscribe_message_gaps()
            .snapshot()
            .unwrap()
            .is_empty()
    );

    assert_eq!(receivers.messages.try_recv().unwrap().sequence, 101);
    assert_eq!(receivers.messages.try_recv().unwrap().sequence, 103);
    assert_eq!(
        receivers.normal_interactions.try_recv().unwrap().sequence,
        102
    );
}

#[test]
fn gi_gap_16_record_at_revision_max_stays_max_merges_and_remains_unacknowledgeable() {
    let (ingress, receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let _ = ingress.publish(
        message_event_at(1_400, 999, "2026-01-01T00:00:00+00:00"),
        Instant::now(),
    );
    let _ = ingress.publish(
        message_event_at(1_401, 490, "2026-01-01T00:00:02+00:00"),
        Instant::now(),
    );
    ingress
        .force_message_gap_limits_for_test(Id::new(490), 7, u64::MAX)
        .unwrap();
    drop(receivers.messages);
    assert_eq!(
        ingress.publish(
            message_event_at(1_399, 490, "2026-01-01T00:00:01+00:00"),
            Instant::now()
        ),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Closed,
            tracking: Ok(())
        }
    );

    let receiver = ingress.subscribe_message_gaps();
    let notice = receiver.snapshot().unwrap().pop().unwrap();
    let gap = notice.snapshot();
    assert_eq!(gap.revision, u64::MAX);
    assert_eq!(gap.observation_count, 8);
    assert_eq!(gap.earliest.message_id, Id::new(1_399));
    assert_eq!(gap.reasons.len(), 2);
    assert!(gap.reasons.contains(UnavailableReason::Full));
    assert!(gap.reasons.contains(UnavailableReason::Closed));
    assert_eq!(
        receiver.acknowledge(notice).unwrap_err(),
        MessageGapAckError::RevisionExhausted
    );
    assert_eq!(receiver.snapshot().unwrap().len(), 1);
}

#[test]
fn gi_gap_17_acknowledgement_surfaces_poison_instead_of_recovering_silently() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    ingress.stop_accepting();
    let _ = ingress.publish(
        message_event_at(1_500, 500, "2026-01-01T00:00:01+00:00"),
        Instant::now(),
    );
    let receiver = ingress.subscribe_message_gaps();
    let notice = receiver.snapshot().unwrap().pop().unwrap();
    ingress.poison_message_gaps_for_test();
    assert_eq!(
        receiver.acknowledge(notice).unwrap_err(),
        MessageGapAckError::StatePoisoned
    );
}

#[test]
fn gi_gap_18_notice_acknowledges_its_exact_channel_not_an_equal_revision_peer() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    ingress.stop_accepting();
    for (id, channel) in [(1_600, 510), (1_601, 511)] {
        let _ = ingress.publish(
            message_event_at(id, channel, "2026-01-01T00:00:01+00:00"),
            Instant::now(),
        );
    }
    let receiver = ingress.subscribe_message_gaps();
    let mut notices = receiver.snapshot().unwrap();
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[0].snapshot().revision, 1);
    assert_eq!(notices[1].snapshot().revision, 1);
    let first = notices[0].snapshot();
    let second = notices.remove(1);

    assert_eq!(
        receiver.acknowledge(second).unwrap(),
        super::ingress::MessageGapAckOutcome::Cleared
    );
    let remaining = receiver.snapshot().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].snapshot(), first);
    assert_eq!(remaining[0].snapshot().channel_id, Id::new(510));
}

#[test]
fn gi_gap_19_current_fence_runs_action_and_pending_gap_refuses_capture() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let gaps = ingress.subscribe_message_gaps();
    let channel_id = Id::<ChannelMarker>::new(520);
    let fence = gaps
        .capture_clear_fence(channel_id)
        .unwrap()
        .expect("clean channel has a fence");
    assert_eq!(gaps.with_current_fence(&fence, || 42), Ok(42));

    ingress.stop_accepting();
    let _ = ingress.publish(
        message_event_at(1_700, channel_id.get(), "2026-01-01T00:00:01+00:00"),
        Instant::now(),
    );
    assert!(gaps.capture_clear_fence(channel_id).unwrap().is_none());
    assert_eq!(
        gaps.with_current_fence(&fence, || 99),
        Err(MessageGapFenceError::Advanced { channel_id })
    );
}

#[test]
fn gi_gap_20_foreign_tracker_fence_cannot_run_the_action() {
    let (first, _first_receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let (second, _second_receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let channel_id = Id::<ChannelMarker>::new(521);
    let fence = first
        .subscribe_message_gaps()
        .capture_clear_fence(channel_id)
        .unwrap()
        .expect("clean channel has a fence");

    assert_eq!(
        second
            .subscribe_message_gaps()
            .with_current_fence(&fence, || 7),
        Err(MessageGapFenceError::ForeignTracker)
    );
}
