use std::{
    sync::{Arc, Barrier},
    thread,
};

use tokio::sync::broadcast;

use super::GatewayIdentityConflict;
use super::gateway_identity::GatewayIdentityTracker;
use super::identity_tests::{identity, publish_ready};

fn no_notification<T>(result: &Result<T, broadcast::error::TryRecvError>) -> bool {
    matches!(result, Err(broadcast::error::TryRecvError::Empty))
}

#[test]
fn gi_06_repeated_identity_is_noop_and_first_fatal_conflict_is_sticky() {
    let first = identity(121, 122);
    let second = identity(131, 132);
    let third = identity(141, 142);
    let identity = GatewayIdentityTracker::new();
    let mut identity_receiver = identity.subscribe_identity();
    let mut conflict_receiver = identity.subscribe_conflict();

    publish_ready(first, &identity);
    assert_eq!(identity_receiver.try_changed().unwrap(), Some(first));
    assert!(no_notification(&conflict_receiver.try_changed()));

    publish_ready(first, &identity);
    assert!(no_notification(&identity_receiver.try_changed()));
    assert!(no_notification(&conflict_receiver.try_changed()));

    let first_conflict = GatewayIdentityConflict {
        established: first,
        observed: second,
    };
    publish_ready(second, &identity);
    assert!(no_notification(&identity_receiver.try_changed()));
    assert_eq!(
        conflict_receiver.try_changed().unwrap(),
        Some(first_conflict)
    );
    assert_eq!(conflict_receiver.snapshot(), Some(first_conflict));

    publish_ready(second, &identity);
    publish_ready(third, &identity);
    assert!(no_notification(&identity_receiver.try_changed()));
    assert!(no_notification(&conflict_receiver.try_changed()));
    assert_eq!(identity_receiver.snapshot(), Some(first));
    assert_eq!(conflict_receiver.snapshot(), Some(first_conflict));
}

#[test]
fn gi_07_concurrent_distinct_ready_events_keep_a_coherent_first_identity_and_conflict() {
    let candidates = [identity(151, 152), identity(161, 162)];
    let identity = GatewayIdentityTracker::new();
    let barrier = Arc::new(Barrier::new(2));
    let workers = candidates.map(|candidate| {
        let identity = identity.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            publish_ready(candidate, &identity);
        })
    });
    for worker in workers {
        worker.join().expect("READY publisher exits");
    }

    let established = identity.subscribe_identity().snapshot().unwrap();
    let conflict = identity.subscribe_conflict().snapshot().unwrap();
    assert!(candidates.contains(&established));
    assert_eq!(conflict.established, established);
    assert!(candidates.contains(&conflict.observed));
    assert_ne!(conflict.observed, established);
}
