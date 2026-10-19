use cdr_store::{
    async_question, async_resolution, delivery, goal_progress, observed_completion, queue, schema,
};
use rusqlite::{Connection, params};
use std::path::Path;

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

const PAYLOAD: &str = r#"{"threadId":"thread-b","turn":{"id":"original","status":"completed"}}"#;

fn legacy_extension(db: &Connection) {
    db.execute_batch(
        "DROP TRIGGER cdr_async_obligation_question;
        DROP TRIGGER cdr_async_obligation_queue_delete;
        DROP TRIGGER cdr_async_obligation_attempt;
        DROP TRIGGER cdr_async_obligation_retention;
        DROP TRIGGER IF EXISTS cdr_async_source_no_delete;
        DROP TRIGGER IF EXISTS cdr_async_source_seal_immutable;
        DROP TRIGGER IF EXISTS cdr_async_uncopied_origin_guard;
        DROP TABLE IF EXISTS cdr_async_terminal_candidates;
        DROP VIEW cdr_async_unsettled_obligations;
        DROP TABLE cdr_async_terminal_settlements;
        DROP TABLE cdr_async_execution_handoffs;
        DROP TABLE cdr_async_execution_obligations;",
    )
    .unwrap();
}

fn correct_observation(path: &Path) {
    observed_completion::record_for_resident(path, "thread-b", "original", 1, PAYLOAD, "resident")
        .unwrap();
}

#[test]
fn r1_accepted_observer_cannot_be_poisoned_in_the_actual_journal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    fixture::dispatching(&path, "resident");
    observed_completion::record_for_resident(
        &path,
        "thread-b",
        "original",
        1,
        PAYLOAD,
        "wrong-resident",
    )
    .unwrap();
    correct_observation(&path);
    assert!(
        observed_completion::has_resident_evidence(&path, "thread-b", "original", 1, "resident")
            .unwrap()
    );
    let owner = queue::list(&path).unwrap().remove(0);
    delivery::stage_owned_queue_completion_with_release(
        &path,
        &owner,
        "result",
        4.0,
        Some(("resident", 1)),
    )
    .unwrap();
    assert!(!async_resolution::admission_held(&path, "thread-b").unwrap());
}

#[test]
fn r1_unverified_candidate_cannot_poison_a_later_exact_notification() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    fixture::dispatching(&path, "resident");
    async_resolution::record_terminal_notification(
        &path,
        "thread-b",
        "original",
        1,
        "wrong-resident",
        PAYLOAD,
    )
    .unwrap();
    correct_observation(&path);
    let owner = queue::list(&path).unwrap().remove(0);
    goal_progress::stage_owned(&path, &owner, "intermediate").unwrap();
    let waiting = queue::list(&path).unwrap().remove(0);
    assert!(queue::attach_goal_turn_observed_if_owned(&path, &waiting, "successor", 1).unwrap());
    assert!(async_resolution::admission_held(&path, "thread-b").unwrap());
}

#[test]
fn r1_malformed_copy_does_not_discard_the_only_journal_in_progress_or_completion() {
    for progress in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite");
        fixture::dispatching(&path, "resident");
        correct_observation(&path);
        schema::open_initialized(&path)
            .unwrap()
            .execute(
                "UPDATE cdr_async_execution_obligations SET terminal_proof_json='{}'",
                [],
            )
            .unwrap();
        let owner = queue::list(&path).unwrap().remove(0);
        if progress {
            goal_progress::stage_owned(&path, &owner, "intermediate").unwrap();
        } else {
            // Compatibility completion has no terminal-settlement authority.
            delivery::stage_owned_queue_completion_with_release(&path, &owner, "result", 4.0, None)
                .unwrap();
        }
        assert!(
            observed_completion::contains(&path, "thread-b", "original").unwrap(),
            "sole journal disappeared, progress={progress}"
        );
        assert!(async_resolution::admission_held(&path, "thread-b").unwrap());
    }
}

#[test]
fn r2_oversized_legacy_claim_is_target_local_and_cannot_be_removed_unsafely() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&path, "resident");
    let db = schema::open_initialized(&path).unwrap();
    legacy_extension(&db);
    let raw: String = db
        .query_row(
            "SELECT preparation_json FROM cdr_async_questions WHERE id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let mut seal: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let prompt = "oversized legacy prompt ".repeat(7000);
    seal["identity"]["job"]["prompt"] = prompt.clone().into();
    let original = serde_json::to_string(&seal).unwrap();
    db.execute(
        "UPDATE codex_turn_queue SET prompt=? WHERE job_id='origin'",
        [prompt],
    )
    .unwrap();
    db.execute(
        "UPDATE cdr_async_questions SET preparation_json=? WHERE id=?",
        params![original, id],
    )
    .unwrap();
    drop(db);
    let db = schema::open_initialized(&path)
        .expect("one oversized claim must not abort every target's migration");
    let after: String = db
        .query_row(
            "SELECT preparation_json FROM cdr_async_questions WHERE id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, original);
    assert!(async_resolution::admission_held(&path, "thread-b").unwrap());
    fixture::pending(&path, "healthy", "unrelated", 1);
    assert!(
        queue::try_begin_attempt(&path, "healthy", &[], 1)
            .unwrap()
            .is_some()
    );
    assert!(queue::complete(&path, "origin").is_err());
    assert!(
        queue::list(&path)
            .unwrap()
            .iter()
            .any(|job| job.job_id == "origin")
    );
    assert!(
        db.execute("DELETE FROM cdr_async_questions WHERE id=?", [&id])
            .is_err()
    );
    assert!(
        db.execute(
            "UPDATE cdr_async_questions SET preparation_json=NULL WHERE id=?",
            [&id]
        )
        .is_err()
    );
}

#[test]
fn r3_combined_journal_guard_accepts_only_the_certified_current_goal_owner() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&path, "resident");
    correct_observation(&path);
    queue::mark_goal_waiting(&path, "origin", "original", 1).unwrap();
    let waiting = queue::list(&path).unwrap().remove(0);
    assert!(queue::attach_goal_turn_observed_if_owned(&path, &waiting, "successor", 1).unwrap());
    async_question::validate_dispatch_guards(&path, "thread-b").unwrap();
    assert!(async_question::confirm_dispatch(&path, &id, "original").is_err());
    assert!(async_question::confirm_dispatch(&path, &id, "successor").is_err());
    assert!(async_resolution::admission_held(&path, "thread-b").unwrap());
    fixture::pending(&path, "later", "thread-b", 1);
    assert!(queue::try_begin_attempt(&path, "later", &[], 1).is_err());
    schema::open_initialized(&path)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET prompt='changed' WHERE job_id='origin'",
            [],
        )
        .unwrap();
    assert!(async_question::validate_dispatch_guards(&path, "thread-b").is_err());
}

#[test]
fn r4_passive_legacy_submitted_updates_never_create_a_new_execution() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let id = fixture::dispatching(&path, "resident");
    let db = schema::open_initialized(&path).unwrap();
    db.execute(
        "UPDATE cdr_async_questions SET state='submitted',accepted_turn_id='original' WHERE id=?",
        [&id],
    )
    .unwrap();
    drop(db);
    queue::complete(&path, "origin").unwrap();
    let db = schema::open_initialized(&path).unwrap();
    legacy_extension(&db);
    drop(db);
    let db = schema::open_initialized(&path).unwrap();
    assert!(!async_resolution::admission_held(&path, "thread-b").unwrap());
    db.execute(
        "UPDATE cdr_async_questions SET error=error WHERE id=?",
        [&id],
    )
    .unwrap();
    db.execute(
        "UPDATE cdr_async_questions SET body='{}',error='retained summary' WHERE id=?",
        [&id],
    )
    .unwrap();
    assert!(!async_resolution::admission_held(&path, "thread-b").unwrap());
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM cdr_async_execution_obligations WHERE question_id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn r5_absent_candidate_preserves_the_no_claim_contract() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    schema::open_initialized(&path).unwrap();
    assert!(
        queue::try_begin_attempt(&path, "already-removed", &[], 1)
            .unwrap()
            .is_none()
    );
    assert!(queue::list(&path).unwrap().is_empty());
}
