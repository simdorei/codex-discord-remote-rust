use tokio::time::Instant;
use twilight_model::gateway::event::Event;

use super::{
    GatewayRuntime,
    ingress::{GatewayIngress, InteractionIngressTag, ReceiveErrorPublishOutcome},
    ingress_tests::{config, interaction_event, message_event},
    runtime_publication::{
        GatewayIngressPublishOutcomes, publish_decoded_event, publish_receive_error,
    },
};

#[tokio::test]
async fn gi_rt_04_pressure_is_visible_without_a_second_event_copy() {
    let mut runtime =
        GatewayRuntime::new_offline_for_test(config(1, 1, 1)).expect("valid runtime config");

    for event in [
        message_event(201),
        message_event(202),
        interaction_event(203),
        interaction_event(204),
        interaction_event(205),
    ] {
        publish_decoded_event(
            event,
            Instant::now(),
            &runtime.ingress,
            &runtime.ingress_publish_outcomes,
        );
    }

    let snapshot = runtime.ingress_publish_outcomes();
    assert_eq!(snapshot.messages_accepted, 1);
    assert_eq!(snapshot.recoverable_message_gaps, 1);
    assert_eq!(snapshot.interactions_accepted, 2);
    assert_eq!(snapshot.hard_dropped_interactions, 1);
    assert_eq!(snapshot.ignored, 0);
    let diagnostics = runtime.subscribe_ingress_diagnostics().snapshot();
    assert_eq!(diagnostics.recoverable_message_gaps, 1);
    assert_eq!(diagnostics.hard_dropped_interactions, 1);

    let mut receivers = runtime.take_ingress_receivers().unwrap();
    assert_eq!(receivers.messages.try_recv().unwrap().event.id.get(), 201);
    let normal = receivers.normal_interactions.try_recv().unwrap();
    assert_eq!(
        (normal.event.id.get(), normal.tag),
        (203, InteractionIngressTag::Normal)
    );
    let reserved = receivers.reserved_interactions.try_recv().unwrap();
    assert_eq!(
        (reserved.event.id.get(), reserved.tag),
        (204, InteractionIngressTag::Busy)
    );
    assert!(receivers.messages.try_recv().is_err());
    assert!(receivers.normal_interactions.try_recv().is_err());
    assert!(receivers.reserved_interactions.try_recv().is_err());
}

#[test]
fn gi_rt_05_receive_error_and_unsupported_event_spend_no_event_sequence() {
    let (ingress, mut receivers) =
        GatewayIngress::new_for_test(config(1, 1, 1), 70).expect("valid config");
    let outcomes = GatewayIngressPublishOutcomes::default();

    assert_eq!(
        publish_receive_error(3, "offline receive failure".into(), &ingress, &outcomes),
        ReceiveErrorPublishOutcome::Accepted
    );
    publish_decoded_event(Event::Resumed, Instant::now(), &ingress, &outcomes);
    publish_decoded_event(interaction_event(206), Instant::now(), &ingress, &outcomes);

    let error = receivers.receive_errors.try_recv().unwrap();
    assert_eq!(
        (error.shard, error.message.as_str()),
        (3, "offline receive failure")
    );
    assert_eq!(
        receivers.normal_interactions.try_recv().unwrap().sequence,
        71
    );
    assert!(receivers.messages.try_recv().is_err());
    assert!(receivers.reserved_interactions.try_recv().is_err());
    let snapshot = outcomes.snapshot();
    assert_eq!(snapshot.ignored, 1);
    assert_eq!(snapshot.receive_errors_accepted, 1);
    assert_eq!(snapshot.receive_errors_dropped, 0);
}

#[test]
fn gi_rt_06_gateway_close_is_ignored_without_allocating_a_sequence() {
    let (ingress, mut receivers) =
        GatewayIngress::new_for_test(config(1, 1, 1), 80).expect("valid config");
    let outcomes = GatewayIngressPublishOutcomes::default();

    publish_decoded_event(
        Event::GatewayClose(None),
        Instant::now(),
        &ingress,
        &outcomes,
    );
    publish_decoded_event(message_event(301), Instant::now(), &ingress, &outcomes);

    assert_eq!(receivers.messages.try_recv().unwrap().sequence, 81);
    assert!(receivers.normal_interactions.try_recv().is_err());
    assert!(receivers.reserved_interactions.try_recv().is_err());
    assert_eq!(outcomes.snapshot().ignored, 1);
}
