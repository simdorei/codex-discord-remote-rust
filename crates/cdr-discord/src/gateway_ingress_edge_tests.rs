use std::{mem::size_of, sync::mpsc, thread, time::Duration};

use super::{
    ingress::{
        GatewayIngress, GatewayIngressConfig, GatewayIngressConfigError, GatewayIngressReceivers,
        IngressDiagnostics, IngressLane, InteractionIngressTag, PublishOutcome, UnavailableReason,
    },
    ingress_tests::{config, interaction_event, message_event, ready_event},
};
use tokio::time::Instant;
use twilight_model::gateway::event::Event;

const INGRESS_SOURCE: &str = include_str!("gateway/ingress.rs");
const INGRESS_TYPES_SOURCE: &str = include_str!("gateway/ingress/types.rs");
const IDENTITY_SOURCE: &str = include_str!("gateway/gateway_identity.rs");

fn publish_promptly(ingress: GatewayIngress, event: Event) -> PublishOutcome {
    let (done, result) = mpsc::sync_channel(1);
    thread::spawn(move || {
        done.send(ingress.publish(event, Instant::now()))
            .expect("test receiver remains open");
    });
    result
        .recv_timeout(Duration::from_secs(1))
        .expect("synchronous publish must not wait for a subscriber")
}

#[test]
fn gi_in_05_zero_capacity_is_rejected_for_each_lane_without_coercion() {
    assert_eq!(GatewayIngressConfig::default(), config(64, 4, 1_024));
    for (config, lane) in [
        (config(0, 1, 1), IngressLane::NormalInteraction),
        (config(1, 0, 1), IngressLane::ReservedInteraction),
        (config(1, 1, 0), IngressLane::Message),
    ] {
        assert_eq!(
            GatewayIngress::new(config).unwrap_err(),
            GatewayIngressConfigError::ZeroCapacity(lane)
        );
    }
}

#[test]
fn gi_in_06_closed_receivers_increment_the_matching_sticky_diagnostic() {
    let (ingress, receivers) = GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    let GatewayIngressReceivers {
        normal_interactions,
        reserved_interactions,
        messages,
        ..
    } = receivers;
    drop(normal_interactions);
    drop(reserved_interactions);
    drop(messages);

    assert_eq!(
        ingress.publish(interaction_event(40), Instant::now()),
        PublishOutcome::InteractionHardDropped {
            reason: UnavailableReason::Closed
        }
    );
    assert_eq!(
        ingress.publish(message_event(41), Instant::now()),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Closed,
            tracking: Ok(())
        }
    );
    let diagnostics = ingress.subscribe_diagnostics();
    assert_eq!(diagnostics.snapshot().hard_dropped_interactions, 1);
    assert_eq!(diagnostics.snapshot().recoverable_message_gaps, 1);
}

#[test]
fn gi_in_07_irrelevant_events_use_no_lane_and_allocate_no_sequence() {
    let (ingress, mut receivers) =
        GatewayIngress::new_for_test(config(1, 1, 1), 70).expect("valid config");

    assert_eq!(
        ingress.publish(Event::Resumed, Instant::now()),
        PublishOutcome::Ignored
    );
    assert_eq!(
        ingress.publish(interaction_event(50), Instant::now()),
        PublishOutcome::InteractionAccepted {
            sequence: 71,
            tag: InteractionIngressTag::Normal
        }
    );
    assert!(receivers.messages.try_recv().is_err());
    assert!(receivers.reserved_interactions.try_recv().is_err());
}

#[test]
fn gi_in_08_sequence_exhaustion_fails_closed_for_both_payload_types() {
    let (ingress, _receivers) =
        GatewayIngress::new_for_test(config(2, 1, 1), u64::MAX - 1).expect("valid config");
    let diagnostics = ingress.subscribe_diagnostics();

    assert_eq!(
        ingress.publish(interaction_event(60), Instant::now()),
        PublishOutcome::InteractionAccepted {
            sequence: u64::MAX,
            tag: InteractionIngressTag::Normal
        }
    );
    assert_eq!(
        ingress.publish(interaction_event(61), Instant::now()),
        PublishOutcome::InteractionHardDropped {
            reason: UnavailableReason::SequenceExhausted
        }
    );
    assert_eq!(
        ingress.publish(message_event(62), Instant::now()),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::SequenceExhausted,
            tracking: Ok(())
        }
    );
    assert_eq!(diagnostics.snapshot().hard_dropped_interactions, 1);
    assert_eq!(diagnostics.snapshot().recoverable_message_gaps, 1);
}

#[test]
fn gi_in_09_process_sequence_is_shared_across_dispatchers_and_received_at_is_exact() {
    let (first, mut first_receivers) = GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    let (second, mut second_receivers) =
        GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    let received_at = Instant::now();
    let PublishOutcome::MessageAccepted { sequence: before } =
        first.publish(message_event(70), received_at)
    else {
        panic!("message accepted");
    };
    let PublishOutcome::InteractionAccepted {
        sequence: after, ..
    } = second.publish(interaction_event(71), received_at)
    else {
        panic!("interaction accepted");
    };

    assert!(after > before);
    assert_eq!(
        first_receivers.messages.try_recv().unwrap().sequence,
        before
    );
    let interaction = second_receivers.normal_interactions.try_recv().unwrap();
    assert_eq!(interaction.sequence, after);
    assert_eq!(interaction.received_at, received_at);
}

#[test]
fn gi_in_10_stopping_reserve_full_is_a_visible_hard_drop() {
    let (ingress, mut receivers) = GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    ingress.stop_accepting();
    assert!(matches!(
        ingress.publish(interaction_event(80), Instant::now()),
        PublishOutcome::InteractionAccepted {
            tag: InteractionIngressTag::Stopping,
            ..
        }
    ));
    assert_eq!(
        ingress.publish(interaction_event(81), Instant::now()),
        PublishOutcome::InteractionHardDropped {
            reason: UnavailableReason::Full
        }
    );
    assert!(receivers.normal_interactions.try_recv().is_err());
    assert_eq!(
        receivers.reserved_interactions.try_recv().unwrap().tag,
        InteractionIngressTag::Stopping
    );
    assert_eq!(
        ingress
            .subscribe_diagnostics()
            .snapshot()
            .hard_dropped_interactions,
        1
    );
}

#[test]
fn gi_in_11_diagnostics_saturate_and_sync_source_has_no_blocking_escape_hatch() {
    assert_eq!(size_of::<IngressDiagnostics>(), size_of::<[u64; 2]>());
    assert!(!INGRESS_SOURCE.contains(".await"));
    assert!(!INGRESS_SOURCE.contains("tokio::spawn"));
    assert!(!INGRESS_SOURCE.contains("spawn_blocking"));
    assert!(!INGRESS_SOURCE.contains("unbounded_channel"));
    assert!(!INGRESS_SOURCE.contains("watch::"));
    assert!(!INGRESS_TYPES_SOURCE.contains("watch::"));
    assert!(!INGRESS_TYPES_SOURCE.contains("send_if_modified"));
    assert!(!IDENTITY_SOURCE.contains("watch::"));
    assert!(!IDENTITY_SOURCE.contains("send_if_modified"));
    assert!(INGRESS_SOURCE.matches(".try_send(").count() >= 3);
}

#[test]
fn gi_in_12_held_snapshots_cannot_stall_ready_or_overflow_publication() {
    let (ready_ingress, _ready_receivers) =
        GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    let identity = ready_ingress.subscribe_identity();
    let held_identity = identity.snapshot();
    assert_eq!(
        publish_promptly(ready_ingress, ready_event(900, 901)),
        PublishOutcome::Ignored
    );
    assert_eq!(held_identity, None);
    assert_eq!(identity.snapshot().unwrap().user_id.get(), 900);

    let (gap_ingress, _gap_receivers) = GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    assert!(matches!(
        gap_ingress.publish(message_event(902), Instant::now()),
        PublishOutcome::MessageAccepted { .. }
    ));
    let diagnostics = gap_ingress.subscribe_diagnostics();
    let held_diagnostics = diagnostics.snapshot();
    assert_eq!(
        publish_promptly(gap_ingress, message_event(903)),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Full,
            tracking: Ok(())
        }
    );
    assert_eq!(held_diagnostics.recoverable_message_gaps, 0);
    assert_eq!(diagnostics.snapshot().recoverable_message_gaps, 1);
}

#[test]
fn gi_in_13_slow_diagnostic_notification_recovers_from_sticky_snapshot() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    let _ = ingress.publish(message_event(910), Instant::now());
    let mut diagnostics = ingress.subscribe_diagnostics();
    for id in 911..928 {
        let _ = ingress.publish(message_event(id), Instant::now());
    }
    assert!(matches!(
        diagnostics.try_changed(),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ));
    assert_eq!(diagnostics.snapshot().recoverable_message_gaps, 17);
}
