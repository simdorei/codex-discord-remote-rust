use std::{
    sync::{Arc, Barrier, mpsc},
    thread,
    time::Duration,
};

use tokio::time::Instant;
use twilight_model::id::Id;

use super::{
    ingress::{
        GatewayIngress, MessageGapAckOutcome, MessageGapStateError, PublishOutcome,
        UnavailableReason,
    },
    ingress_tests::{config, message_event_at},
};

const GAP_SOURCE: &str = include_str!("gateway/ingress/gaps.rs");
const INGRESS_SOURCE: &str = include_str!("gateway/ingress.rs");
const TYPES_SOURCE: &str = include_str!("gateway/ingress/types.rs");

#[test]
fn gi_gap_07_concurrent_channels_are_all_preserved() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let _ = ingress.publish(
        message_event_at(600, 999, "2026-01-01T00:00:00+00:00"),
        Instant::now(),
    );
    let barrier = Arc::new(Barrier::new(17));
    let workers: Vec<_> = (0..16)
        .map(|offset| {
            let ingress = ingress.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                ingress.publish(
                    message_event_at(601 + offset, 400 + offset, "2026-01-01T00:00:01+00:00"),
                    Instant::now(),
                )
            })
        })
        .collect();
    barrier.wait();
    for worker in workers {
        assert!(matches!(
            worker.join().unwrap(),
            PublishOutcome::MessageRecoverableGap {
                reason: UnavailableReason::Full,
                tracking: Ok(())
            }
        ));
    }

    let notices = ingress.subscribe_message_gaps().snapshot().unwrap();
    assert_eq!(notices.len(), 16);
    let channels: Vec<_> = notices
        .iter()
        .map(|notice| notice.snapshot().channel_id.get())
        .collect();
    assert_eq!(channels, (400..416).collect::<Vec<_>>());
}

#[test]
fn gi_gap_08_lagged_hint_recovers_every_observation_from_snapshot() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let _ = ingress.publish(
        message_event_at(700, 999, "2026-01-01T00:00:00+00:00"),
        Instant::now(),
    );
    let mut receiver = ingress.subscribe_message_gaps();
    for id in 701..718 {
        let _ = ingress.publish(
            message_event_at(id, 420, "2026-01-01T00:00:01+00:00"),
            Instant::now(),
        );
    }
    assert!(matches!(
        receiver.try_changed(),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ));
    let gap = receiver.snapshot().unwrap().pop().unwrap().snapshot();
    assert_eq!(gap.observation_count, 17);
    assert_eq!(gap.revision, 17);
}

#[test]
fn gi_gap_09_slow_subscriber_cannot_block_synchronous_publication() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let _held_receiver = ingress.subscribe_message_gaps();
    let _ = ingress.publish(
        message_event_at(800, 999, "2026-01-01T00:00:00+00:00"),
        Instant::now(),
    );
    let (done, result) = mpsc::sync_channel(1);
    thread::spawn(move || {
        for id in 801..1_001 {
            let _ = ingress.publish(
                message_event_at(id, 430, "2026-01-01T00:00:01+00:00"),
                Instant::now(),
            );
        }
        done.send(()).unwrap();
    });
    result
        .recv_timeout(Duration::from_secs(1))
        .expect("bounded hints and short locks must not wait for a subscriber");
}

#[test]
fn gi_gap_11_observation_count_saturates_without_freezing_normal_revision() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let _ = ingress.publish(
        message_event_at(1_000, 999, "2026-01-01T00:00:00+00:00"),
        Instant::now(),
    );
    let _ = ingress.publish(
        message_event_at(1_001, 450, "2026-01-01T00:00:01+00:00"),
        Instant::now(),
    );
    ingress
        .force_message_gap_limits_for_test(Id::new(450), u64::MAX, 40)
        .unwrap();
    let _ = ingress.publish(
        message_event_at(1_002, 450, "2026-01-01T00:00:02+00:00"),
        Instant::now(),
    );
    let gap = ingress
        .subscribe_message_gaps()
        .snapshot()
        .unwrap()
        .pop()
        .unwrap()
        .snapshot();
    assert_eq!(gap.observation_count, u64::MAX);
    assert_eq!(gap.revision, 41);
}

#[test]
fn gi_gap_12_poisoned_state_is_visible_in_publish_and_snapshot() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    let mut receiver = ingress.subscribe_message_gaps();
    ingress.poison_message_gaps_for_test();
    ingress.stop_accepting();
    assert_eq!(
        ingress.publish(
            message_event_at(1_100, 460, "2026-01-01T00:00:01+00:00"),
            Instant::now()
        ),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Stopping,
            tracking: Err(MessageGapStateError::Poisoned)
        }
    );
    assert_eq!(
        receiver.snapshot().unwrap_err(),
        MessageGapStateError::Poisoned
    );
    assert_eq!(
        ingress
            .subscribe_diagnostics()
            .snapshot()
            .recoverable_message_gaps,
        1
    );
    assert_eq!(receiver.try_changed(), Ok(()));
}

#[test]
fn gi_gap_13_source_uses_explicit_failures_not_mixed_sequence_gaps_or_message_clones() {
    assert!(!GAP_SOURCE.contains("sequence"));
    assert!(!GAP_SOURCE.contains("message.clone("));
    assert!(!GAP_SOURCE.contains("event.clone("));
    assert!(GAP_SOURCE.lines().count() <= 250);
    assert!(INGRESS_SOURCE.lines().count() <= 250);
    let publish_message = INGRESS_SOURCE
        .split("fn publish_message")
        .nth(1)
        .unwrap()
        .split("fn record_hard_drop")
        .next()
        .unwrap();
    assert!(!publish_message.contains(".clone("));
    let publication = publish_message
        .find("begin_publication()")
        .expect("message publication fence");
    let send = publish_message
        .find("messages.try_send")
        .expect("message queue attempt");
    let record = publish_message
        .find("publication.record(event, reason)")
        .expect("gap record bound to publication fence");
    assert!(publication < send && send < record);
    assert!(
        !TYPES_SOURCE.contains("#[derive(Clone, Debug, PartialEq)]\npub struct MessageIngress")
    );
}

#[test]
fn gi_gap_14_ack_after_clear_is_stale_and_never_resets_revision() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    ingress.stop_accepting();
    let _ = ingress.publish(
        message_event_at(1_200, 470, "2026-01-01T00:00:01+00:00"),
        Instant::now(),
    );
    let receiver = ingress.subscribe_message_gaps();
    let first = receiver.snapshot().unwrap().pop().unwrap();
    let duplicate = receiver.snapshot().unwrap().pop().unwrap();
    assert_eq!(
        receiver.acknowledge(first).unwrap(),
        MessageGapAckOutcome::Cleared
    );
    assert_eq!(
        receiver.acknowledge(duplicate).unwrap(),
        MessageGapAckOutcome::Stale
    );
    let _ = ingress.publish(
        message_event_at(1_201, 470, "2026-01-01T00:00:02+00:00"),
        Instant::now(),
    );
    assert_eq!(
        receiver
            .snapshot()
            .unwrap()
            .pop()
            .unwrap()
            .snapshot()
            .revision,
        2
    );
}
