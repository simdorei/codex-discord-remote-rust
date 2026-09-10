use std::sync::{Arc, Barrier};
use std::thread;

use cdr_store::claims::{
    NewBusyChoice, busy_choice_counts, claim_busy_choice, claim_component,
    cleanup_component_claims, component_claim_counts, create_busy_choice, get_busy_choice,
    release_busy_choice_claim, release_component_claim,
};
use cdr_store::schema::open_initialized;

#[test]
fn c1_concurrent_component_clicks_have_exactly_one_winner() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("claims.sqlite");
    drop(open_initialized(&path).expect("initialize claim store"));

    let workers = 12;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|_| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                claim_component(&path, "message:42:button:approve", 100.0, 60.0)
            })
        })
        .collect::<Vec<_>>();

    let winners = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("claim worker does not panic")
                .expect("claim worker succeeds")
        })
        .filter(|claimed| *claimed)
        .count();
    assert_eq!(winners, 1);
}

#[test]
fn c3_failed_component_action_can_release_its_claim_for_retry() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("claims.sqlite");
    assert!(claim_component(&path, "component", 100.0, 60.0).unwrap());
    assert!(release_component_claim(&path, "component").unwrap());
    assert!(!release_component_claim(&path, "component").unwrap());
    assert!(claim_component(&path, "component", 101.0, 60.0).unwrap());

    let choice = create_busy_choice(
        &path,
        NewBusyChoice {
            owner_user_id: 1,
            channel_id: 2,
            target_thread_id: Some("thread"),
            prompt: "retry",
            allow_steer: true,
            now: 100.0,
            time_to_live: 60.0,
        },
    )
    .unwrap();
    assert!(claim_busy_choice(&path, &choice, 101.0).unwrap());
    assert!(release_busy_choice_claim(&path, &choice).unwrap());
    assert!(claim_busy_choice(&path, &choice, 102.0).unwrap());
}

#[test]
fn c2_busy_and_component_claims_expire_and_remain_one_use() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("claims.sqlite");

    assert!(claim_component(&path, "component", 100.0, 10.0).expect("first claim"));
    assert!(!claim_component(&path, "component", 105.0, 10.0).expect("live duplicate"));
    assert_eq!(
        component_claim_counts(&path, 105.0).expect("claim counts"),
        (1, 0)
    );
    assert!(claim_component(&path, "component", 110.0, 10.0).expect("expired key reused"));
    assert_eq!(
        cleanup_component_claims(&path, 120.0).expect("cleanup component"),
        1
    );

    let choice_id = create_busy_choice(
        &path,
        NewBusyChoice {
            owner_user_id: 7,
            channel_id: 8,
            target_thread_id: Some("thread-1"),
            prompt: "hello",
            allow_steer: true,
            now: 200.0,
            time_to_live: 10.0,
        },
    )
    .expect("create busy choice");
    let choice = get_busy_choice(&path, &choice_id, 205.0)
        .expect("read busy choice")
        .expect("busy choice is active");
    assert_eq!(choice.owner_user_id, 7);
    assert_eq!(choice.target_thread_id.as_deref(), Some("thread-1"));
    assert!(choice.allow_steer);
    assert!(claim_busy_choice(&path, &choice_id, 205.0).expect("first busy claim"));
    assert!(!claim_busy_choice(&path, &choice_id, 206.0).expect("second busy claim"));
    assert_eq!(
        busy_choice_counts(&path, 206.0).expect("busy counts"),
        (0, 1)
    );
    assert_eq!(
        get_busy_choice(&path, &choice_id, 206.0).expect("claimed read"),
        None
    );
}
