use std::{
    cell::{Cell, RefCell},
    sync::{Arc, Barrier},
    thread,
};

use tokio::time::Instant;

use super::{
    ingress::{GatewayIngress, InteractionIngressTag, PublishOutcome},
    ingress_tests::{config, interaction_event, message_event, ready_event},
    runtime_publication::{
        GatewayIngressPublishOutcomes, publish_decoded_event, publish_decoded_event_with,
    },
};

const GATEWAY_SOURCE: &str = include_str!("gateway.rs");
const PUBLICATION_SOURCE: &str = include_str!("gateway/runtime_publication.rs");

#[test]
fn gi_rt_09_runtime_and_typed_publication_have_one_identity_source() {
    assert!(!GATEWAY_SOURCE.contains("identity: GatewayIdentityTracker"));
    assert!(!PUBLICATION_SOURCE.contains("publish_gateway_event_identity"));
}

#[test]
fn gi_rt_02_message_and_interaction_move_only_to_their_typed_lanes() {
    let (ingress, mut receivers) =
        GatewayIngress::new_for_test(config(2, 1, 2), 40).expect("valid config");
    let outcomes = GatewayIngressPublishOutcomes::default();
    let message_received_at = Instant::now();
    let interaction_received_at = message_received_at + std::time::Duration::from_millis(7);

    publish_decoded_event(message_event(101), message_received_at, &ingress, &outcomes);
    publish_decoded_event(
        interaction_event(102),
        interaction_received_at,
        &ingress,
        &outcomes,
    );

    let message = receivers.messages.try_recv().expect("message lane item");
    let interaction = receivers
        .normal_interactions
        .try_recv()
        .expect("normal interaction lane item");
    assert_eq!((message.sequence, message.event.id.get()), (41, 101));
    assert_eq!(
        (interaction.sequence, interaction.event.id.get()),
        (42, 102)
    );
    assert_eq!(interaction.received_at, interaction_received_at);
    assert_eq!(interaction.tag, InteractionIngressTag::Normal);
    assert!(receivers.reserved_interactions.try_recv().is_err());
    assert_eq!(outcomes.snapshot().messages_accepted, 1);
    assert_eq!(outcomes.snapshot().interactions_accepted, 1);
}

#[test]
fn gi_rt_03_ready_is_sticky_before_typed_and_outcome_observers() {
    let expected = super::identity_tests::identity(501, 601);
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    let identity = ingress.subscribe_identity();
    let order = RefCell::new(Vec::new());

    publish_decoded_event_with(
        ready_event(501, 601),
        Instant::now(),
        &ingress,
        || {
            assert_eq!(identity.snapshot(), Some(expected));
            order.borrow_mut().push("typed");
        },
        |outcome| {
            assert_eq!(outcome, PublishOutcome::Ignored);
            order.borrow_mut().push("outcome");
        },
    );

    assert_eq!(*order.borrow(), ["typed", "outcome"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gi_rt_10_concurrent_ready_observers_share_one_identity_and_conflict() {
    let runtime =
        super::GatewayRuntime::new_offline_for_test(config(1, 1, 1)).expect("valid config");
    let public_identity = runtime.subscribe_identity();
    let public_conflict = runtime.subscribe_identity_conflict();
    let barrier = Arc::new(Barrier::new(2));

    let workers = [(701, 801), (702, 802)].map(|(user_id, application_id)| {
        let ingress = runtime.ingress.clone();
        let identity = runtime.subscribe_identity();
        let conflict = runtime.subscribe_identity_conflict();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let observed = Cell::new(None);
            publish_decoded_event_with(
                ready_event(user_id, application_id),
                Instant::now(),
                &ingress,
                || {
                    barrier.wait();
                    observed.set(Some((identity.snapshot(), conflict.snapshot())));
                },
                |_| {},
            );
            observed.get().unwrap()
        })
    });

    let observations = workers.map(|worker| worker.join().expect("READY publisher exits"));
    let established = public_identity.snapshot().expect("one READY wins");
    let conflict = public_conflict.snapshot().expect("other READY conflicts");
    assert_eq!(conflict.established, established);
    for observed in observations {
        assert_eq!(observed, (Some(established), Some(conflict)));
    }
}
