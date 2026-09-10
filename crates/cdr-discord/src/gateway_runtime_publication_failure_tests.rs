use tokio::time::Instant;

use super::{
    ingress::GatewayIngress,
    ingress_tests::{config, interaction_event, message_event},
    runtime_publication::{GatewayIngressPublishOutcomes, publish_decoded_event},
};

const PUBLICATION_SOURCE: &str = include_str!("gateway/runtime_publication.rs");
const SHARD_SOURCE: &str = include_str!("gateway/shard.rs");

#[test]
fn gi_rt_07_sequence_exhaustion_is_visible_and_fail_closed() {
    let (ingress, mut receivers) =
        GatewayIngress::new_for_test(config(1, 1, 1), u64::MAX).expect("valid config");
    let outcomes = GatewayIngressPublishOutcomes::default();
    publish_decoded_event(interaction_event(401), Instant::now(), &ingress, &outcomes);
    publish_decoded_event(message_event(402), Instant::now(), &ingress, &outcomes);

    assert!(receivers.normal_interactions.try_recv().is_err());
    assert!(receivers.reserved_interactions.try_recv().is_err());
    assert!(receivers.messages.try_recv().is_err());
    assert_eq!(outcomes.snapshot().hard_dropped_interactions, 1);
    assert_eq!(outcomes.snapshot().recoverable_message_gaps, 1);
}

#[test]
fn gi_rt_08_publication_helper_has_no_wait_clone_or_blocking_primitives() {
    for forbidden in [
        ".await",
        "event.clone()",
        "tokio::spawn",
        "spawn_blocking",
        "unbounded_channel",
        "Mutex",
        "RwLock",
        "thread::",
    ] {
        assert!(
            !PUBLICATION_SOURCE.contains(forbidden),
            "publication source contains forbidden primitive {forbidden}"
        );
    }
}

#[test]
fn gi_rt_12_shard_wires_events_and_receive_errors_to_typed_publishers() {
    let compact = SHARD_SOURCE.split_whitespace().collect::<String>();
    assert_eq!(compact.matches("publish_decoded_event(").count(), 1);
    assert!(compact.contains(concat!(
        "publish_decoded_event(event,Instant::now(),&ingress,",
        "&ingress_publish_outcomes,);"
    )));
    assert_eq!(compact.matches("publish_receive_error(").count(), 1);
    assert!(compact.contains(concat!(
        "publish_receive_error(shard_id,error.to_string(),&ingress,",
        "&ingress_publish_outcomes,);"
    )));
    assert!(!compact.contains("broadcast::Sender"));
}
