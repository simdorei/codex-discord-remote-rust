//! Additional ordering/corruption/Goal oracles for the approved recovery plan.
use cdr_store::{
    async_question as aq, async_resolution as resolution, delivery, queue, schema::open_initialized,
};

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

fn observe(db: &std::path::Path, turn: &str) {
    let payload =
        serde_json::json!({"threadId":"thread-b","turn":{"id":turn,"status":"completed"}})
            .to_string();
    resolution::record_terminal_notification(db, "thread-b", turn, 1, "resident", &payload)
        .unwrap();
}

fn finish(db: &std::path::Path) {
    let before = queue::list(db).unwrap().remove(0);
    delivery::stage_owned_queue_completion_with_release(
        db,
        &before,
        "Final",
        4.0,
        Some(("resident", 1)),
    )
    .unwrap();
}

#[test]
fn late_answer_ack_does_not_strand_a_verified_ordinary_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&db, "resident");
    observe(&db, "original");
    aq::confirm_dispatch(&db, &id, "original").unwrap();
    finish(&db);
    assert_eq!(aq::get(&db, &id).unwrap().state, "submitted");
    assert!(
        !resolution::admission_held(&db, "thread-b").unwrap(),
        "receipt-only revision must not discard exact terminal evidence"
    );
}

#[test]
fn malformed_settled_evidence_cannot_authorize_a_new_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::dispatching(&db, "resident");
    observe(&db, "original");
    finish(&db);
    open_initialized(&db)
        .unwrap()
        .execute(
            "UPDATE cdr_async_execution_obligations SET terminal_proof_json='{}'",
            [],
        )
        .unwrap();
    let held = resolution::admission_held(&db, "thread-b");
    assert!(
        held.is_err() || held.unwrap(),
        "non-null is not verified evidence"
    );
    fixture::pending(&db, "next", "thread-b", 2);
    assert!(queue::try_begin_attempt(&db, "next", &[], 2).is_err());
}

#[test]
fn verified_goal_successor_preserves_then_settles_the_original_obligation() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::dispatching(&db, "resident");
    observe(&db, "original");
    assert!(queue::mark_goal_waiting(&db, "origin", "original", 1).unwrap());
    assert!(resolution::admission_held(&db, "thread-b").unwrap());
    let waiting = queue::list(&db).unwrap().remove(0);
    assert!(queue::attach_goal_turn_observed_if_owned(&db, &waiting, "successor", 1).unwrap());
    assert!(resolution::admission_held(&db, "thread-b").unwrap());
    observe(&db, "successor");
    finish(&db);
    assert!(
        !resolution::admission_held(&db, "thread-b").unwrap(),
        "verified logical completion must not leave a permanent ordinary hold"
    );
}
