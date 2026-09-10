use tokio::time::Instant;
use twilight_model::{
    gateway::event::Event,
    id::{
        Id,
        marker::{ChannelMarker, MessageMarker},
    },
};

use super::{
    ingress::{
        GatewayIngress, MessageGapAckError, MessageGapAckOutcome, MessageGapSnapshot,
        PublishOutcome, UnavailableReason,
    },
    ingress_tests::{config, message_event_at},
};

fn event_position(event: &Event) -> (Id<ChannelMarker>, Id<MessageMarker>, i64) {
    let Event::MessageCreate(message) = event else {
        panic!("test event must be MessageCreate");
    };
    (
        message.channel_id,
        message.id,
        message.timestamp.as_micros(),
    )
}

fn only_gap(ingress: &GatewayIngress) -> MessageGapSnapshot {
    let notices = ingress
        .subscribe_message_gaps()
        .snapshot()
        .expect("message gap state remains healthy");
    assert_eq!(notices.len(), 1);
    notices.into_iter().next().unwrap().snapshot()
}

fn assert_gap_reason(
    ingress: &GatewayIngress,
    expected: (Id<ChannelMarker>, Id<MessageMarker>, i64),
    reason: UnavailableReason,
) {
    let gap = only_gap(ingress);
    assert_eq!(gap.channel_id, expected.0);
    assert_eq!(gap.earliest.message_id, expected.1);
    assert_eq!(gap.earliest.timestamp_micros, expected.2);
    assert_eq!(gap.observation_count, 1);
    assert_eq!(gap.reasons.len(), 1);
    assert!(gap.reasons.contains(reason));
}

#[test]
fn gi_gap_01_every_failure_reason_records_the_exact_message_position() {
    let full_event = message_event_at(101, 301, "2026-01-01T00:00:01.123456+00:00");
    let full_position = event_position(&full_event);
    let (full, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    assert!(matches!(
        full.publish(
            message_event_at(100, 999, "2026-01-01T00:00:00+00:00"),
            Instant::now()
        ),
        PublishOutcome::MessageAccepted { .. }
    ));
    assert_eq!(
        full.publish(full_event, Instant::now()),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Full,
            tracking: Ok(())
        }
    );
    assert_gap_reason(&full, full_position, UnavailableReason::Full);

    let closed_event = message_event_at(102, 302, "2026-01-01T00:00:02.234567+00:00");
    let closed_position = event_position(&closed_event);
    let (closed, receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    drop(receivers.messages);
    assert_eq!(
        closed.publish(closed_event, Instant::now()),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Closed,
            tracking: Ok(())
        }
    );
    assert_gap_reason(&closed, closed_position, UnavailableReason::Closed);

    let stopping_event = message_event_at(103, 303, "2026-01-01T00:00:03.345678+00:00");
    let stopping_position = event_position(&stopping_event);
    let (stopping, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    stopping.stop_accepting();
    assert_eq!(
        stopping.publish(stopping_event, Instant::now()),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Stopping,
            tracking: Ok(())
        }
    );
    assert_gap_reason(&stopping, stopping_position, UnavailableReason::Stopping);

    let exhausted_event = message_event_at(104, 304, "2026-01-01T00:00:04.456789+00:00");
    let exhausted_position = event_position(&exhausted_event);
    let (exhausted, _receivers) = GatewayIngress::new_for_test(config(1, 1, 1), u64::MAX).unwrap();
    assert_eq!(
        exhausted.publish(exhausted_event, Instant::now()),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::SequenceExhausted,
            tracking: Ok(())
        }
    );
    assert_gap_reason(
        &exhausted,
        exhausted_position,
        UnavailableReason::SequenceExhausted,
    );
}

#[test]
fn gi_gap_02_channel_gap_keeps_lexicographic_earliest_floor_and_combines_reasons() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let _ = ingress.publish(
        message_event_at(200, 999, "2026-01-01T00:00:00+00:00"),
        Instant::now(),
    );
    for (id, timestamp) in [
        (230, "2026-01-01T00:00:03+00:00"),
        (240, "2026-01-01T00:00:02+00:00"),
    ] {
        let _ = ingress.publish(message_event_at(id, 310, timestamp), Instant::now());
    }
    ingress.stop_accepting();
    let _ = ingress.publish(
        message_event_at(235, 310, "2026-01-01T00:00:02+00:00"),
        Instant::now(),
    );

    let gap = only_gap(&ingress);
    assert_eq!(gap.channel_id, Id::new(310));
    assert_eq!(gap.earliest.message_id, Id::new(235));
    assert_eq!(gap.earliest.timestamp_micros, 1_767_225_602_000_000);
    assert_eq!(gap.observation_count, 3);
    assert_eq!(gap.reasons.len(), 2);
    assert!(gap.reasons.contains(UnavailableReason::Full));
    assert!(gap.reasons.contains(UnavailableReason::Stopping));
}

#[test]
fn gi_gap_03_no_subscriber_and_late_subscriber_recover_authoritative_state() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let _ = ingress.publish(
        message_event_at(300, 999, "2026-01-01T00:00:00+00:00"),
        Instant::now(),
    );
    let _ = ingress.publish(
        message_event_at(301, 311, "2026-01-01T00:00:01+00:00"),
        Instant::now(),
    );

    let mut late = ingress.subscribe_message_gaps();
    assert_eq!(late.snapshot().unwrap().len(), 1);
    assert!(matches!(
        late.try_changed(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

#[test]
fn gi_gap_04_stale_and_duplicate_snapshot_acknowledgements_cannot_erase_new_work() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let _ = ingress.publish(
        message_event_at(400, 999, "2026-01-01T00:00:00+00:00"),
        Instant::now(),
    );
    let _ = ingress.publish(
        message_event_at(401, 312, "2026-01-01T00:00:01+00:00"),
        Instant::now(),
    );
    let receiver = ingress.subscribe_message_gaps();
    let old = receiver.snapshot().unwrap().pop().unwrap();
    let duplicate_old = receiver.snapshot().unwrap().pop().unwrap();
    let old_revision = old.snapshot().revision;

    let _ = ingress.publish(
        message_event_at(402, 312, "2026-01-01T00:00:02+00:00"),
        Instant::now(),
    );
    assert_eq!(
        receiver.acknowledge(old).unwrap(),
        MessageGapAckOutcome::Stale
    );
    assert_eq!(only_gap(&ingress).revision, old_revision + 1);

    assert_eq!(
        receiver.acknowledge(duplicate_old).unwrap(),
        MessageGapAckOutcome::Stale
    );
    let current = receiver.snapshot().unwrap().pop().unwrap();
    let duplicate_current = receiver.snapshot().unwrap().pop().unwrap();
    assert_eq!(
        receiver.acknowledge(current).unwrap(),
        MessageGapAckOutcome::Cleared
    );
    assert_eq!(
        receiver.acknowledge(duplicate_current).unwrap(),
        MessageGapAckOutcome::Stale
    );
    assert!(receiver.snapshot().unwrap().is_empty());

    let _ = ingress.publish(
        message_event_at(403, 312, "2026-01-01T00:00:03+00:00"),
        Instant::now(),
    );
    assert_eq!(only_gap(&ingress).revision, old_revision + 2);
}

#[test]
fn gi_gap_06_cross_tracker_notice_is_rejected_without_touching_either_tracker() {
    let (first, _first_receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let (second, _second_receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    first.stop_accepting();
    let _ = first.publish(
        message_event_at(500, 313, "2026-01-01T00:00:01+00:00"),
        Instant::now(),
    );
    let first_receiver = first.subscribe_message_gaps();
    let foreign_notice = first_receiver.snapshot().unwrap().pop().unwrap();

    assert_eq!(
        second
            .subscribe_message_gaps()
            .acknowledge(foreign_notice)
            .unwrap_err(),
        MessageGapAckError::ForeignTracker
    );
    assert_eq!(first_receiver.snapshot().unwrap().len(), 1);
    assert!(
        second
            .subscribe_message_gaps()
            .snapshot()
            .unwrap()
            .is_empty()
    );
}
