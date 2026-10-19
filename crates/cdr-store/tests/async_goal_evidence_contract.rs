use cdr_store::{async_resolution, goal_progress, observed_completion, queue, schema};
use rusqlite::params;

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

const PAYLOAD: &str = r#"{"threadId":"thread-b","turn":{"id":"original","status":"completed"}}"#;

#[test]
fn goal_progress_preserves_the_only_uncopied_terminal_journal() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::dispatching(&db, "resident");
    observed_completion::record(&db, "thread-b", "original", 1, PAYLOAD).unwrap();
    let owner = queue::list(&db).unwrap().remove(0);
    goal_progress::stage_owned(&db, &owner, "progress only").unwrap();
    let count: i64 = schema::open_initialized(&db).unwrap().query_row(
        "SELECT COUNT(*) FROM codex_observed_completions WHERE thread_id='thread-b' AND turn_id='original'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(
        count, 1,
        "Goal progress must not erase the sole original terminal evidence"
    );
    assert!(async_resolution::admission_held(&db, "thread-b").unwrap());
    let waiting = queue::list(&db).unwrap().remove(0);
    assert!(waiting.goal_waiting);
    assert!(queue::attach_goal_turn_observed_if_owned(&db, &waiting, "next", 1).is_err());
    assert_eq!(queue::list(&db).unwrap()[0], waiting);
}

#[test]
fn copied_goal_terminal_can_be_compacted_but_not_logically_settled() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::dispatching(&db, "resident");
    async_resolution::record_terminal_notification(
        &db, "thread-b", "original", 1, "resident", PAYLOAD,
    )
    .unwrap();
    observed_completion::record_for_resident(&db, "thread-b", "original", 1, PAYLOAD, "resident")
        .unwrap();
    let owner = queue::list(&db).unwrap().remove(0);
    goal_progress::stage_owned(&db, &owner, "progress only").unwrap();
    let count: i64 = schema::open_initialized(&db).unwrap().query_row(
        "SELECT COUNT(*) FROM codex_observed_completions WHERE thread_id='thread-b' AND turn_id='original'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 0);
    assert!(async_resolution::admission_held(&db, "thread-b").unwrap());
    let waiting = queue::list(&db).unwrap().remove(0);
    assert!(queue::attach_goal_turn_observed_if_owned(&db, &waiting, "next", 1).unwrap());
    assert!(async_resolution::admission_held(&db, "thread-b").unwrap());
    async_resolution::guard_mutation(&db, "thread-b").unwrap();
    fixture::pending(&db, "later", "thread-b", 1);
    assert!(queue::try_begin_attempt(&db, "later", &[], 1).is_err());
}

#[test]
fn failed_goal_evidence_commit_rolls_back_the_existing_owner_cas() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&db, "resident");
    async_resolution::record_terminal_notification(
        &db, "thread-b", "original", 1, "resident", PAYLOAD,
    )
    .unwrap();
    queue::mark_goal_waiting(&db, "origin", "original", 1).unwrap();
    let waiting = queue::list(&db).unwrap().remove(0);
    let connection = schema::open_initialized(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_goal_handoff BEFORE INSERT ON cdr_async_execution_handoffs BEGIN SELECT RAISE(ABORT,'injected handoff failure'); END").unwrap();
    assert!(queue::attach_goal_turn_observed_if_owned(&db, &waiting, "next", 1).is_err());
    assert_eq!(queue::list(&db).unwrap()[0], waiting);
    let (revision, proof): (i64, Option<String>) = connection.query_row(
        "SELECT revision,terminal_proof_json FROM cdr_async_execution_obligations WHERE question_id=?",
        params![id], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(revision, 0);
    assert!(proof.is_some());
}

#[test]
fn mutated_goal_input_cannot_inherit_the_original_execution_authority() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::dispatching(&db, "resident");
    async_resolution::record_terminal_notification(
        &db, "thread-b", "original", 1, "resident", PAYLOAD,
    )
    .unwrap();
    queue::mark_goal_waiting(&db, "origin", "original", 1).unwrap();
    schema::open_initialized(&db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET prompt='different input' WHERE job_id='origin'",
            [],
        )
        .unwrap();
    let changed = queue::list(&db).unwrap().remove(0);
    assert!(queue::attach_goal_turn_observed_if_owned(&db, &changed, "next", 1).is_err());
    assert_eq!(queue::list(&db).unwrap()[0], changed);
    assert!(async_resolution::admission_held(&db, "thread-b").unwrap());
}
