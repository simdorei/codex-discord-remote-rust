//! Revision 14: exact owner CAS includes progress, waiting handoff and both journals.
use cdr_store::{goal_progress, observed_completion, observed_final_answer, queue};
use std::path::Path;

fn running(db: &Path, id: &str, turn: &str, generation: i64) -> queue::StoredQueueJob {
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: id,
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "original",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, id, &["baseline".into()], generation).unwrap();
    queue::mark_running(db, id, turn, generation).unwrap();
    queue::list(db)
        .unwrap()
        .into_iter()
        .find(|job| job.job_id == id)
        .unwrap()
}

fn journal(db: &Path) {
    assert!(observed_completion::record(db, "thread", "T2", 1, "terminal").unwrap());
    assert!(observed_final_answer::record(db, "thread", "T2", 1, "exact").unwrap());
}

fn assert_no_handoff(db: &Path) {
    assert!(goal_progress::pending(db).unwrap().is_empty());
    assert!(observed_completion::contains(db, "thread", "T2").unwrap());
    assert_eq!(
        observed_final_answer::get(db, "thread", "T2", 1).unwrap(),
        Some("exact".into())
    );
    assert!(
        !cdr_store::mirror::has_event(
            db,
            &cdr_store::mirror::turn_origin_marker("thread", "T2"),
            "thread"
        )
        .unwrap()
    );
}

#[test]
fn revision14_owned_progress_rejects_every_stale_snapshot_without_consuming_journals() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let current = running(&db, "job", "T2", 1);
    journal(&db);
    let mut snapshots = Vec::new();
    let mut stale = current.clone();
    stale.owner_user_id = Some(99);
    snapshots.push(stale);
    let mut stale = current.clone();
    stale.turn_observation_generation = Some(3);
    snapshots.push(stale);
    let mut stale = current.clone();
    stale.prompt = "changed".into();
    snapshots.push(stale);
    let mut stale = current.clone();
    stale.goal_waiting = true;
    snapshots.push(stale);
    for stale in snapshots {
        assert!(goal_progress::stage_owned(&db, &stale, "wrong").is_err());
        assert_eq!(
            queue::list(&db).unwrap().as_slice(),
            std::slice::from_ref(&current)
        );
        assert_no_handoff(&db);
    }
}

#[test]
fn revision14_owned_progress_rejects_multiple_running_generations() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let owner = running(&db, "job", "T2", 1);
    journal(&db);
    running(&db, "other", "other-turn", 2);
    let before = queue::list(&db).unwrap();
    assert!(goal_progress::stage_owned(&db, &owner, "wrong").is_err());
    assert_eq!(queue::list(&db).unwrap(), before);
    assert_no_handoff(&db);
}

#[test]
fn revision14_owned_progress_transaction_failure_preserves_owner_and_journals() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let owner = running(&db, "job", "T2", 1);
    journal(&db);
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_handoff BEFORE UPDATE OF goal_waiting ON codex_turn_queue
        BEGIN SELECT RAISE(ABORT,'test handoff failure'); END;",
        )
        .unwrap();
    assert!(
        goal_progress::stage_owned(&db, &owner, "correct")
            .unwrap_err()
            .to_string()
            .contains("test handoff failure")
    );
    assert_eq!(queue::list(&db).unwrap(), [owner]);
    assert_no_handoff(&db);
}

#[test]
fn revision14_owned_progress_commits_exact_payload_and_preserves_execution() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let owner = running(&db, "job", "T2", 1);
    journal(&db);
    let progress = goal_progress::stage_owned(&db, &owner, "[Goal progress]\nexact")
        .unwrap()
        .unwrap();
    assert_eq!(progress.content, "[Goal progress]\nexact");
    let mut expected = owner.clone();
    expected.goal_waiting = true;
    assert_eq!(queue::list(&db).unwrap(), [expected]);
    assert!(!observed_completion::contains(&db, "thread", "T2").unwrap());
    assert_eq!(
        observed_final_answer::get(&db, "thread", "T2", 1).unwrap(),
        None
    );
    assert!(goal_progress::stage_owned(&db, &owner, "changed").is_err());
    assert_eq!(
        goal_progress::pending(&db).unwrap()[0].content,
        progress.content
    );
}
