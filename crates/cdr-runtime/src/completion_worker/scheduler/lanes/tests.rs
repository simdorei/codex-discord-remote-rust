use std::collections::HashSet;
use std::sync::Arc;

use cdr_app_server::Notification;
use serde_json::json;

use super::*;

fn event(target: &str, method: &str, budget: &Arc<Semaphore>) -> Envelope {
    Envelope::charge(
        ResidentNotificationEvent::Notification {
            generation: 1,
            notification: Notification {
                method: method.into(),
                params: json!({"threadId":target,"turn":{"id":"turn","status":"completed"}}),
            },
        },
        budget,
    )
    .unwrap()
}

#[test]
fn ready_count_target_count_and_idle_keys_are_bounded() {
    let budget = Arc::new(Semaphore::new(EVENT_BYTES));
    let mut ready = Ready::default();
    for _ in 0..TARGET_CAP {
        assert!(ready.live(event("a", "turn/started", &budget)));
    }
    assert!(!ready.live(event("a", "turn/started", &budget)));
    for n in TARGET_CAP..READY_CAP {
        assert!(ready.live(event(&format!("target-{n}"), "turn/started", &budget)));
    }
    assert_eq!(ready.len(), READY_CAP);
    assert!(!ready.live(event("overflow", "turn/started", &budget)));
    while ready.take_state(&HashSet::new(), 0).is_some() {}
    assert_eq!(ready.len(), 0);
    assert_eq!(budget.available_permits(), EVENT_BYTES);
}

#[test]
fn native_saturation_reserves_a_state_slot_for_local_terminal_commit() {
    let budget = Arc::new(Semaphore::new(EVENT_BYTES));
    let mut ready = Ready::default();
    assert!(ready.live(event("a", "turn/completed", &budget)));
    assert!(ready.live(event("b", "turn/started", &budget)));
    let item = ready.take_state(&HashSet::new(), NATIVE_SLOTS).unwrap();
    assert_eq!(item.target(), "b");
    assert_eq!(ready.state.len(), 1);
}

#[test]
fn state_fifo_excludes_active_target_without_blocking_another_target() {
    let budget = Arc::new(Semaphore::new(EVENT_BYTES));
    let mut ready = Ready::default();
    for (target, method) in [("a", "first"), ("a", "second"), ("b", "other")] {
        assert!(ready.live(event(target, method, &budget)));
    }
    let first = ready.take_state(&HashSet::new(), 0).unwrap();
    assert_eq!(first.target(), "a");
    let b = ready.take_state(&HashSet::from(["a".into()]), 0).unwrap();
    assert_eq!(b.target(), "b");
    let StateWork::Live(second) = ready.take_state(&HashSet::new(), 0).unwrap() else {
        panic!("live");
    };
    let ResidentNotificationEvent::Notification { notification, .. } = second.event else {
        panic!("event");
    };
    assert_eq!(notification.method, "second");
}

#[test]
fn live_payload_budget_is_shared_until_the_last_owner_drops() {
    let budget = Arc::new(Semaphore::new(128));
    let held = event("a", "turn/started", &budget);
    assert!(budget.available_permits() < 128);
    let huge = ResidentNotificationEvent::Notification {
        generation: 1,
        notification: Notification {
            method: "item/completed".into(),
            params: json!({"threadId":"b","text":"x".repeat(129)}),
        },
    };
    assert!(Envelope::charge(huge, &budget).is_none());
    drop(held);
    assert_eq!(budget.available_permits(), 128);
}

#[test]
fn patch05_review_p1_native_saturation_cannot_overtake_same_target_head() {
    let budget = Arc::new(Semaphore::new(EVENT_BYTES));
    let mut ready = Ready::default();
    for (target, method) in [
        ("a", "turn/completed"),
        ("a", "turn/started"),
        ("b", "turn/started"),
    ] {
        assert!(ready.live(event(target, method, &budget)));
    }
    let other = ready.take_state(&HashSet::new(), NATIVE_SLOTS).unwrap();
    assert_eq!(
        other.target(),
        "b",
        "A start must not overtake its blocked completion"
    );
    assert!(ready.take_state(&HashSet::new(), NATIVE_SLOTS).is_none());
    let StateWork::Live(first) = ready.take_state(&HashSet::new(), NATIVE_SLOTS - 1).unwrap()
    else {
        panic!("live completion");
    };
    let ResidentNotificationEvent::Notification { notification, .. } = &first.event else {
        panic!("notification");
    };
    assert_eq!(notification.method, "turn/completed");
    assert!(ready.take_state(&HashSet::from(["a".into()]), 0).is_none());
    let StateWork::Live(next) = ready.take_state(&HashSet::new(), 0).unwrap() else {
        panic!("live start");
    };
    let ResidentNotificationEvent::Notification { notification, .. } = &next.event else {
        panic!("notification");
    };
    assert_eq!(notification.method, "turn/started");
    drop((first, next, other));
    assert_eq!(budget.available_permits(), EVENT_BYTES);
}
