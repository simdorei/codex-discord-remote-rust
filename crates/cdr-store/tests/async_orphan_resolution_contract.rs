//! Behavioral regression: acceptance is not terminal execution evidence.
use cdr_store::{async_question as aq, queue, schema::open_initialized};

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

#[test]
fn orphan_dispatch_is_a_target_hold_not_a_missing_queue_error() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&db, "resident");
    let connection = open_initialized(&db).unwrap();
    let columns = connection
        .prepare("PRAGMA table_info(cdr_async_questions)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    println!("fixture_question_columns={columns:?}");
    let seal: String = connection
        .query_row(
            "SELECT preparation_json FROM cdr_async_questions WHERE id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    println!("fixture_original_preparation={seal}");
    drop(connection);
    queue::complete(&db, "origin").unwrap();
    let error = aq::validate_dispatch_guards(&db, "thread-b")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("[cdr-rust:async-resolution-held:v1]"),
        "{error}"
    );
    assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
}

#[test]
fn accepted_answer_without_terminal_cannot_lose_target_protection() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&db, "resident");
    aq::confirm_dispatch(&db, &id, "original").unwrap();
    queue::complete(&db, "origin").unwrap();
    // Reopening and changing generation are not proof that execution ended.
    fixture::pending(&db, "next", "thread-b", 2);
    let attempt = queue::try_begin_attempt(&db, "next", &[], 2);
    assert!(
        attempt.is_err() || attempt.as_ref().is_ok_and(Option::is_none),
        "receipt alone allowed a new attempt: {attempt:?}"
    );
    assert_eq!(aq::get(&db, &id).unwrap().state, "submitted");
    let held = cdr_store::room_cleanup::pending_reason(&db, 20, Some("thread-b")).unwrap();
    assert!(held.is_some());
    fixture::pending(&db, "unrelated", "unrelated", 2);
    assert!(
        queue::try_begin_attempt(&db, "unrelated", &[], 2)
            .unwrap()
            .is_some()
    );
}
