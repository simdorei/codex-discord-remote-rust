use cdr_store::{
    async_question as aq, async_resolution as resolution, delivery, queue, schema::open_initialized,
};

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

const TERMINAL: &str = r#"{"threadId":"thread-b","turn":{"id":"original","status":"completed"}}"#;

fn observe(db: &std::path::Path) {
    resolution::record_terminal_notification(db, "thread-b", "original", 1, "resident", TERMINAL)
        .unwrap();
}

#[test]
fn ordinary_exact_owned_terminal_settles_without_a_pending_job() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&db, "resident");
    aq::validate_dispatch_guards(&db, "thread-b").unwrap();
    observe(&db);
    let before = queue::list(&db).unwrap().remove(0);
    delivery::stage_owned_queue_completion_with_release(
        &db,
        &before,
        "Final",
        4.0,
        Some(("resident", 1)),
    )
    .unwrap();
    assert!(queue::list(&db).unwrap().is_empty());
    assert_eq!(aq::get(&db, &id).unwrap().state, "closed_unknown");
    assert!(!resolution::admission_held(&db, "thread-b").unwrap());
    assert!(resolution::inspect(&db, "thread-b").unwrap().is_empty());
    fixture::pending(&db, "next", "thread-b", 1);
    assert!(
        queue::try_begin_attempt(&db, "next", &[], 1)
            .unwrap()
            .is_some()
    );
}

#[test]
fn publishing_policy_stays_held_after_exact_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::dispatching(&db, "resident");
    open_initialized(&db)
        .unwrap()
        .execute(
            "UPDATE cdr_async_execution_obligations SET policy='publishing_recovery'",
            [],
        )
        .unwrap();
    observe(&db);
    let before = queue::list(&db).unwrap().remove(0);
    delivery::stage_owned_queue_completion_with_release(
        &db,
        &before,
        "Final",
        4.0,
        Some(("resident", 1)),
    )
    .unwrap();
    let rows = resolution::inspect(&db, "thread-b").unwrap();
    assert_eq!(rows[0].execution_state, "terminal");
    assert_eq!(rows[0].admission_state, "held");
    assert!(resolution::admission_held(&db, "thread-b").unwrap());
}

#[test]
fn generic_removal_or_unowned_completion_cannot_borrow_observed_terminal() {
    for owned_api_without_provenance in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        fixture::dispatching(&db, "resident");
        observe(&db);
        if owned_api_without_provenance {
            let before = queue::list(&db).unwrap().remove(0);
            delivery::stage_owned_queue_completion_with_release(&db, &before, "Final", 4.0, None)
                .unwrap();
        } else {
            queue::complete(&db, "origin").unwrap();
        }
        assert!(resolution::admission_held(&db, "thread-b").unwrap());
        fixture::pending(&db, "next", "thread-b", 2);
        assert!(queue::try_begin_attempt(&db, "next", &[], 2).is_err());
    }
}

#[test]
fn settlement_failure_rolls_back_question_queue_and_outbox_together() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&db, "resident");
    observe(&db);
    let before = queue::list(&db).unwrap().remove(0);
    open_initialized(&db).unwrap().execute_batch(
        "CREATE TRIGGER reject_settlement BEFORE UPDATE OF execution_state ON cdr_async_execution_obligations
         BEGIN SELECT RAISE(ABORT,'injected settlement failure'); END;",
    ).unwrap();
    assert!(
        delivery::stage_owned_queue_completion_with_release(
            &db,
            &before,
            "Final",
            4.0,
            Some(("resident", 1))
        )
        .is_err()
    );
    assert_eq!(queue::list(&db).unwrap(), [before]);
    assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
    assert!(delivery::list_pending(&db).unwrap().is_empty());
}

#[test]
fn wrong_turn_status_or_observer_is_not_settlement_authority() {
    for (observer, payload) in [
        ("other", TERMINAL),
        (
            "resident",
            r#"{"threadId":"thread-b","turn":{"id":"wrong","status":"completed"}}"#,
        ),
        (
            "resident",
            r#"{"threadId":"thread-b","turn":{"id":"original","status":"inProgress"}}"#,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        fixture::dispatching(&db, "resident");
        let _ = resolution::record_terminal_notification(
            &db, "thread-b", "original", 1, observer, payload,
        );
        let before = queue::list(&db).unwrap().remove(0);
        delivery::stage_owned_queue_completion_with_release(
            &db,
            &before,
            "Final",
            4.0,
            Some(("resident", 1)),
        )
        .unwrap();
        assert!(resolution::admission_held(&db, "thread-b").unwrap());
    }
}
