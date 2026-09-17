//! Revision 12: persisted Goal owner CAS is independent of the live resident.
use cdr_store::{goal_progress, queue};
use std::path::Path;

fn setup(db: &Path, id: &str, generation: i64) -> queue::StoredQueueJob {
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: id,
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "original input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, id, &["baseline".into()], generation).unwrap();
    queue::mark_running(db, id, "T1", generation).unwrap();
    queue::mark_goal_waiting(db, id, "T1", generation).unwrap();
    queue::list(db)
        .unwrap()
        .into_iter()
        .find(|j| j.job_id == id)
        .unwrap()
}

#[test]
fn exact_inherited_owner_attaches_without_rewriting_execution_evidence() {
    for legacy in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        setup(&db, "job", 1);
        if legacy {
            rusqlite::Connection::open(&db)
                .unwrap()
                .execute(
                    "UPDATE codex_turn_queue SET execution_generation=NULL WHERE job_id='job'",
                    [],
                )
                .unwrap();
        }
        queue::adopt_generation(&db, 2).unwrap();
        let before = queue::list(&db).unwrap().remove(0);
        assert_eq!(before.app_server_generation, 1);
        assert!(queue::attach_goal_turn_if_owned(&db, &before, "T2").unwrap());
        let after = queue::list(&db).unwrap().remove(0);
        assert_eq!(after.turn_id.as_deref(), Some("T2"));
        assert!(!after.goal_waiting);
        assert_eq!(after.app_server_generation, before.app_server_generation);
        assert_eq!(after.execution_generation, before.execution_generation);
        assert_eq!(after.attempt_count, before.attempt_count);
        assert_eq!(after.baseline_turn_ids, before.baseline_turn_ids);
        assert_eq!(after.prompt, before.prompt);
    }
}

#[test]
fn mismatched_owner_fields_do_not_authorize_a_goal_attachment() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let owner = setup(&db, "job", 1);
    let mut wrong = vec![owner.clone(); 11];
    wrong[0].job_id = "foreign-job".into();
    wrong[1].target_thread_id = "foreign-thread".into();
    wrong[2].turn_id = Some("foreign-turn".into());
    wrong[3].app_server_generation = 2;
    wrong[4].execution_generation = None;
    wrong[5].attempt_count += 1;
    wrong[6].channel_id += 1;
    wrong[7].owner_user_id = Some(999);
    wrong[8].goal_waiting = false;
    wrong[9].state = queue::QueueJobState::Starting;
    wrong[10].turn_id = None;
    for expected in wrong {
        assert!(!queue::attach_goal_turn_if_owned(&db, &expected, "T2").unwrap());
        assert_eq!(
            queue::list(&db).unwrap().as_slice(),
            std::slice::from_ref(&owner)
        );
    }
    for turn in ["", " ", "T1"] {
        assert!(!queue::attach_goal_turn_if_owned(&db, &owner, turn).unwrap());
    }
}

#[test]
fn stale_waiting_snapshot_cannot_attach_after_a_later_handoff() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let first = setup(&db, "job", 1);
    goal_progress::stage(&db, "job", "T1", 1, "progress 1").unwrap();
    let first = queue::list(&db)
        .unwrap()
        .into_iter()
        .find(|j| j.job_id == first.job_id)
        .unwrap();
    assert!(queue::attach_goal_turn_if_owned(&db, &first, "T2").unwrap());
    goal_progress::stage(&db, "job", "T2", 1, "progress 2").unwrap();
    let second = queue::list(&db).unwrap().remove(0);
    assert!(!queue::attach_goal_turn_if_owned(&db, &first, "T3").unwrap());
    // A delayed completed-turn start cannot rewind the current waiting job.
    assert!(!queue::attach_goal_turn_if_owned(&db, &second, "T1").unwrap());
    assert_eq!(
        queue::list(&db).unwrap().as_slice(),
        std::slice::from_ref(&second)
    );
    assert!(queue::attach_goal_turn_if_owned(&db, &second, "T3").unwrap());
    assert!(!queue::attach_goal_turn_if_owned(&db, &second, "T4").unwrap());
    assert_eq!(goal_progress::pending(&db).unwrap().len(), 2);
}

#[test]
fn duplicate_running_owners_across_generations_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let first = setup(&db, "first", 1);
    setup(&db, "second", 2);
    let before = queue::list(&db).unwrap();
    let error = queue::attach_goal_turn_if_owned(&db, &first, "T2").unwrap_err();
    assert!(error.to_string().contains("multiple running jobs"));
    assert_eq!(queue::list(&db).unwrap(), before);
}

#[test]
fn removed_original_owner_is_not_replaced_by_another_waiting_job() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let first = setup(&db, "first", 1);
    queue::complete(&db, "first").unwrap();
    let second = setup(&db, "second", 1);
    assert!(!queue::attach_goal_turn_if_owned(&db, &first, "T2").unwrap());
    assert_eq!(queue::list(&db).unwrap(), [second]);
}

#[test]
fn dead_generation_hold_blocks_the_exact_owner_path() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    cdr_store::dead_generation::activate_runtime(&db, "runtime").unwrap();
    let owner = setup(&db, "job", 1);
    cdr_store::dead_generation::capture_dead_generation(
        &db,
        cdr_store::dead_generation::DeadGenerationCapture {
            runtime_id: "runtime",
            generation: 1,
            snapshot_json: "{}",
            affected_targets: &["thread".into()],
            startup_channel_id: Some(42),
            has_unscoped_requests: false,
            now: 3.0,
        },
    )
    .unwrap();
    let before = queue::list(&db).unwrap();
    assert!(!queue::attach_goal_turn_if_owned(&db, &owner, "T2").unwrap());
    assert_eq!(queue::list(&db).unwrap(), before);
}

#[test]
fn failed_attachment_transaction_preserves_waiting_owner_and_progress() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    setup(&db, "job", 1);
    goal_progress::stage(&db, "job", "T1", 1, "original progress").unwrap();
    let owner = queue::list(&db).unwrap().remove(0);
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_attach BEFORE UPDATE OF turn_id ON codex_turn_queue
         BEGIN SELECT RAISE(ABORT,'injected attachment failure'); END;",
        )
        .unwrap();
    let error = queue::attach_goal_turn_if_owned(&db, &owner, "T2").unwrap_err();
    assert!(error.to_string().contains("injected attachment failure"));
    assert_eq!(queue::list(&db).unwrap(), [owner]);
    let pending = goal_progress::pending(&db).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "original progress");
}
