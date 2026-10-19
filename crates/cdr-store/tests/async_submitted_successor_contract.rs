//! PATCH-01 review regression: receipt state cannot weaken live owner guards.
use cdr_store::{
    async_question as aq, async_resolution, delivery, queue, schema::open_initialized,
};
use std::path::Path;

#[path = "../../../tests/support/submitted_successor_fixture.rs"]
mod fixture;

fn held_without_mutation(db: &Path, id: &str) {
    let before = fixture::snapshot(db);
    assert!(
        aq::validate_dispatch_guards(db, "thread-b").is_err(),
        "ACK-first execution bypassed a common target guard"
    );
    assert_eq!(fixture::snapshot(db), before);
    assert_eq!(aq::get(db, id).unwrap().state, "submitted");
}

#[test]
fn ack_first_successor_preserves_archive_fences() {
    for phase in ["attempted", "verified"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let id = fixture::successor(&db, "resident");
        open_initialized(&db).unwrap().execute(
            "INSERT INTO codex_archive_fences(target_thread_id,operation_id,phase) VALUES('thread-b','archive',?)",
            [phase],
        ).unwrap();
        held_without_mutation(&db, &id);
    }
}

#[test]
fn ack_first_successor_preserves_cleanup_fences() {
    for phase in ["deleting", "deleted"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let id = fixture::successor(&db, "resident");
        open_initialized(&db).unwrap().execute(
            "INSERT INTO cdr_cleanup_fences(channel_id,target_thread_id,token,phase,created_at) VALUES(20,'thread-b','cleanup',?,4)",
            [phase],
        ).unwrap();
        held_without_mutation(&db, &id);
    }
}

#[test]
fn ack_first_successor_preserves_dead_generation_fence() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::successor(&db, "resident");
    open_initialized(&db)
        .unwrap()
        .execute(
            "INSERT INTO codex_dead_generation_holds VALUES('thread-b','previous-resident',0,4)",
            [],
        )
        .unwrap();
    held_without_mutation(&db, &id);
}

#[test]
fn ack_first_successor_rejects_duplicate_mapping() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::successor(&db, "resident");
    open_initialized(&db)
        .unwrap()
        .execute(
            "INSERT INTO mirror_threads VALUES('other-thread','project','other',10,20,4)",
            [],
        )
        .unwrap();
    held_without_mutation(&db, &id);
}

#[test]
fn ack_first_successor_rejects_channel_fallback_mapping() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::successor(&db, "resident");
    open_initialized(&db).unwrap().execute(
        "UPDATE mirror_threads SET discord_thread_id=21,discord_channel_id=20 WHERE codex_thread_id='thread-b'", [],
    ).unwrap();
    held_without_mutation(&db, &id);
}

#[test]
fn ack_first_successor_rejects_competing_nonpending_owners() {
    for generation in [1, 2] {
        for state in ["starting", "running", "quarantined"] {
            let temp = tempfile::tempdir().unwrap();
            let db = temp.path().join("state.sqlite");
            let id = fixture::successor(&db, "resident");
            fixture::pending(&db, "competitor", "unrelated", generation);
            queue::begin_attempt(&db, "competitor", &[], generation).unwrap();
            if state != "starting" {
                queue::mark_running(&db, "competitor", "other-turn", generation).unwrap();
            }
            let connection = open_initialized(&db).unwrap();
            if state == "quarantined" {
                connection.execute(
                    "UPDATE codex_turn_queue SET turn_id='cdr-quarantined:fixture',last_error='[cdr-rust:app-server-fork-quarantine:v1] fixture' WHERE job_id='competitor'", [],
                ).unwrap();
            }
            aq::validate_dispatch_guards(&db, "thread-b").unwrap();
            // Fault injection: normal admission must not create this competing
            // execution on the already held target.
            connection.execute(
                "UPDATE codex_turn_queue SET target_thread_id='thread-b' WHERE job_id='competitor'", [],
            ).unwrap();
            held_without_mutation(&db, &id);
        }
    }
}

#[test]
fn ack_first_original_execution_preserves_common_guards() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::submitted(&db, "resident");
    aq::validate_dispatch_guards(&db, "thread-b").unwrap();
    open_initialized(&db).unwrap().execute(
        "INSERT INTO codex_archive_fences(target_thread_id,operation_id,phase) VALUES('thread-b','archive','attempted')", [],
    ).unwrap();
    held_without_mutation(&db, &id);
}

#[test]
fn ack_first_successor_pending_and_settled_history_keep_distinct_authority() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let id = fixture::successor(&db, "resident");
    fixture::pending(&db, "next", "thread-b", 2);
    let before = fixture::snapshot(&db);
    aq::validate_dispatch_guards(&db, "thread-b").unwrap();
    assert!(aq::confirm_dispatch(&db, &id, "original").is_err());
    assert!(aq::confirm_dispatch(&db, &id, "successor").is_err());
    assert!(aq::reject_definite(&db, &id, "late rejection").is_err());
    assert!(queue::try_begin_attempt(&db, "next", &[], 2).is_err());
    assert_eq!(fixture::snapshot(&db), before);
    assert!(async_resolution::admission_held(&db, "thread-b").unwrap());

    fixture::observe(&db, "resident", "successor");
    let current = queue::list(&db)
        .unwrap()
        .into_iter()
        .find(|job| job.job_id == "origin")
        .unwrap();
    delivery::stage_owned_queue_completion_with_release(
        &db,
        &current,
        "Final",
        6.0,
        Some(("resident", 1)),
    )
    .unwrap();
    assert!(
        queue::list(&db)
            .unwrap()
            .iter()
            .all(|job| job.job_id != "origin")
    );
    assert!(!async_resolution::admission_held(&db, "thread-b").unwrap());
    assert_eq!(aq::get(&db, &id).unwrap().state, "submitted");
    let settled = fixture::snapshot(&db);
    aq::validate_dispatch_guards(&db, "thread-b").unwrap();
    assert_eq!(fixture::snapshot(&db), settled);
    queue::try_begin_attempt(&db, "next", &[], 2).unwrap();
    assert_eq!(
        queue::list(&db).unwrap()[0].state,
        queue::QueueJobState::Starting
    );
}
