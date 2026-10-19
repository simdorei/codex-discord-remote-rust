use cdr_store::{async_question, async_resolution, observed_completion, queue, schema};
use rusqlite::params;
use std::path::Path;

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

fn successor(path: &Path) -> String {
    let id = fixture::dispatching(path, "resident");
    observed_completion::record_for_resident(
        path,
        "thread-b",
        "original",
        1,
        r#"{"threadId":"thread-b","turn":{"id":"original","status":"completed"}}"#,
        "resident",
    )
    .unwrap();
    queue::mark_goal_waiting(path, "origin", "original", 1).unwrap();
    let waiting = queue::list(path).unwrap().remove(0);
    assert!(queue::attach_goal_turn_observed_if_owned(path, &waiting, "successor", 1).unwrap());
    async_question::validate_dispatch_guards(path, "thread-b").unwrap();
    id
}

fn assert_held_without_mutation(path: &Path, id: &str) {
    let jobs = queue::list(path).unwrap();
    let db = schema::open_initialized(path).unwrap();
    let seal: String = db
        .query_row(
            "SELECT preparation_json FROM cdr_async_questions WHERE id=?",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let result = async_question::validate_dispatch_guards(path, "thread-b");
    assert!(
        result.is_err(),
        "certified successor bypassed a common target guard"
    );
    assert_eq!(queue::list(path).unwrap(), jobs);
    let after: String = db
        .query_row(
            "SELECT preparation_json FROM cdr_async_questions WHERE id=?",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, seal);
    assert_eq!(async_question::get(path, id).unwrap().state, "dispatching");
}

#[test]
fn b0_01_certified_successor_preserves_archive_fences() {
    for phase in ["attempted", "verified"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite");
        let id = successor(&path);
        schema::open_initialized(&path)
            .unwrap()
            .execute(
                "INSERT INTO codex_archive_fences(target_thread_id,operation_id,phase)
                 VALUES('thread-b','archive-intent',?)",
                [phase],
            )
            .unwrap();
        assert_held_without_mutation(&path, &id);
    }
}

#[test]
fn b0_02_certified_successor_preserves_cleanup_fences() {
    for phase in ["deleting", "deleted"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite");
        let id = successor(&path);
        schema::open_initialized(&path)
            .unwrap()
            .execute(
                "INSERT INTO cdr_cleanup_fences(channel_id,target_thread_id,token,phase,created_at)
                 VALUES(20,'thread-b','cleanup-intent',?,6.0)",
                [phase],
            )
            .unwrap();
        assert_held_without_mutation(&path, &id);
    }
}

#[test]
fn b0_03_certified_successor_preserves_dead_generation_fence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let id = successor(&path);
    schema::open_initialized(&path)
        .unwrap()
        .execute(
            "INSERT INTO codex_dead_generation_holds
             (target_thread_id,runtime_id,generation,created_at)
             VALUES('thread-b','previous-resident',0,6.0)",
            [],
        )
        .unwrap();
    assert_held_without_mutation(&path, &id);
}

#[test]
fn b0_04_duplicate_mapping_does_not_borrow_successor_authority() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let id = successor(&path);
    schema::open_initialized(&path)
        .unwrap()
        .execute(
            "INSERT INTO mirror_threads VALUES('different-thread','project','other',10,20,0)",
            [],
        )
        .unwrap();
    assert_held_without_mutation(&path, &id);
}

#[test]
fn b0_04_channel_fallback_is_not_the_exact_thread_mapping() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let id = successor(&path);
    schema::open_initialized(&path)
        .unwrap()
        .execute(
            "UPDATE mirror_threads SET discord_thread_id=21,discord_channel_id=20
             WHERE codex_thread_id='thread-b'",
            [],
        )
        .unwrap();
    assert_held_without_mutation(&path, &id);
}

#[test]
fn b0_05_competing_nonpending_owner_is_rejected_across_generations() {
    for generation in [1, 2] {
        for state in ["starting", "running", "quarantined"] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("state.sqlite");
            let id = successor(&path);
            fixture::pending(&path, "competing", "unrelated", generation);
            queue::begin_attempt(&path, "competing", &[], generation).unwrap();
            if state == "running" {
                queue::mark_running(&path, "competing", "another-turn", generation).unwrap();
            } else if state == "quarantined" {
                // Quarantined is persisted as a running row with both markers,
                // not as a literal SQL state named "quarantined".
                schema::open_initialized(&path)
                    .unwrap()
                    .execute(
                        "UPDATE codex_turn_queue SET state='running',
                         turn_id='cdr-quarantined:competing',
                         last_error='[cdr-rust:app-server-fork-quarantine:v1] fixture'
                         WHERE job_id='competing'",
                        [],
                    )
                    .unwrap();
            }
            // A different target's current execution must not affect this one.
            async_question::validate_dispatch_guards(&path, "thread-b").unwrap();
            // Inject competing durable ownership, without making a new attempt
            // on the held target or weakening its existing attempt trigger.
            schema::open_initialized(&path)
                .unwrap()
                .execute(
                    "UPDATE codex_turn_queue SET target_thread_id='thread-b' WHERE job_id='competing'",
                    [],
                )
                .unwrap();
            assert_held_without_mutation(&path, &id);
        }
    }
}

#[test]
fn b0_06_successor_exception_preserves_original_answer_and_pending_start_guards() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let id = successor(&path);
    fixture::pending(&path, "next", "thread-b", 2);
    let before = queue::list(&path).unwrap();
    let db = schema::open_initialized(&path).unwrap();
    let seal: String = db
        .query_row(
            "SELECT preparation_json FROM cdr_async_questions WHERE id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    // Pending work is not another current owner; the certified execution may finish.
    async_question::validate_dispatch_guards(&path, "thread-b").unwrap();
    assert!(async_question::confirm_dispatch(&path, &id, "original").is_err());
    assert!(async_question::confirm_dispatch(&path, &id, "successor").is_err());
    assert!(async_question::reject_definite(&path, &id, "late rejection").is_err());
    assert!(queue::try_begin_attempt(&path, "next", &[], 2).is_err());
    assert!(async_resolution::admission_held(&path, "thread-b").unwrap());
    assert_eq!(queue::list(&path).unwrap(), before);
    let unchanged: bool = db
        .query_row(
            "SELECT preparation_json=? AND turn_id='original' AND chosen=0
             AND state='dispatching' FROM cdr_async_questions WHERE id=?",
            params![seal, id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(unchanged);
}
