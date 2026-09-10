use std::{
    cell::Cell,
    sync::{Arc, Barrier},
    thread,
};

use tokio::time::Instant;

use super::{
    ingress::{GatewayIngress, InteractionIngressTag, PublishOutcome},
    ingress_tests::{config, interaction_event, message_event},
    runtime_publication::{
        GatewayIngressPublishOutcomes, publish_decoded_event, publish_decoded_event_with,
    },
};

const WORKERS: usize = 8;
const EVENTS_PER_WORKER: usize = 8;
const EVENT_COUNT: usize = WORKERS * EVENTS_PER_WORKER;

#[test]
fn gi_rt_11_concurrent_publication_keeps_unique_sequences_and_exact_counters() {
    let (ingress, mut receivers) =
        GatewayIngress::new_for_test(config(64, 4, 64), 500).expect("valid config");
    let outcomes = GatewayIngressPublishOutcomes::default();
    let barrier = Arc::new(Barrier::new(WORKERS));

    let workers = (0..WORKERS)
        .map(|worker| {
            let ingress = ingress.clone();
            let outcomes = outcomes.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for offset in 0..EVENTS_PER_WORKER {
                    let index = worker * EVENTS_PER_WORKER + offset;
                    let id = 1_000 + u64::try_from(index).unwrap();
                    let event = if index.is_multiple_of(2) {
                        message_event(id)
                    } else {
                        interaction_event(id)
                    };
                    publish_decoded_event(event, Instant::now(), &ingress, &outcomes);
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("publisher exits");
    }

    let snapshot = outcomes.snapshot();
    assert_eq!(
        (snapshot.messages_accepted, snapshot.interactions_accepted),
        (32, 32)
    );
    assert_eq!(
        (
            snapshot.recoverable_message_gaps,
            snapshot.hard_dropped_interactions
        ),
        (0, 0)
    );
    let mut sequences = Vec::with_capacity(EVENT_COUNT);
    let mut ids = Vec::with_capacity(EVENT_COUNT);
    while let Ok(message) = receivers.messages.try_recv() {
        sequences.push(message.sequence);
        ids.push(message.event.id.get());
    }
    while let Ok(interaction) = receivers.normal_interactions.try_recv() {
        sequences.push(interaction.sequence);
        ids.push(interaction.event.id.get());
    }
    assert!(receivers.reserved_interactions.try_recv().is_err());
    sequences.sort_unstable();
    ids.sort_unstable();
    assert_eq!(sequences, (501..=564).collect::<Vec<_>>());
    assert_eq!(ids, (1_000..1_064).collect::<Vec<_>>());
}

#[test]
fn gi_rt_13_stop_accepting_uses_the_acceptance_read_as_its_cutoff() {
    let (ingress, mut receivers) =
        GatewayIngress::new_for_test(config(2, 1, 1), 700).expect("valid config");
    let outcomes = GatewayIngressPublishOutcomes::default();
    publish_decoded_event(interaction_event(901), Instant::now(), &ingress, &outcomes);

    let observer_entered = Arc::new(Barrier::new(2));
    let continue_publication = Arc::new(Barrier::new(2));
    let worker_ingress = ingress.clone();
    let worker_entered = Arc::clone(&observer_entered);
    let worker_continue = Arc::clone(&continue_publication);
    let worker = thread::spawn(move || {
        let outcome = Cell::new(None);
        publish_decoded_event_with(
            interaction_event(902),
            Instant::now(),
            &worker_ingress,
            || {
                worker_entered.wait();
                worker_continue.wait();
            },
            |observed| outcome.set(Some(observed)),
        );
        outcome.get().unwrap()
    });

    observer_entered.wait();
    ingress.stop_accepting();
    continue_publication.wait();
    assert!(matches!(
        worker.join().expect("publisher exits"),
        PublishOutcome::InteractionAccepted {
            tag: InteractionIngressTag::Stopping,
            ..
        }
    ));
    assert_eq!(
        receivers
            .normal_interactions
            .try_recv()
            .unwrap()
            .event
            .id
            .get(),
        901
    );
    let stopping = receivers.reserved_interactions.try_recv().unwrap();
    assert_eq!(
        (stopping.event.id.get(), stopping.tag),
        (902, InteractionIngressTag::Stopping)
    );
}
