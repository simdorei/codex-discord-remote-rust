//! GH2: durable handoff is atomic; GH3: retries cannot replace the payload.
use cdr_store::{goal_progress, observed_completion, queue};

fn setup(db: &std::path::Path) {
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: 7,
            prompt: "input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, "job", &[], 7).unwrap();
    queue::mark_running(db, "job", "T1", 7).unwrap();
    observed_completion::record(db, "thread", "T1", 7, "terminal").unwrap();
}

#[test]
fn handoff_storage_failure_rolls_back_payload_waiting_and_journal_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    setup(&db);
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TRIGGER fail_handoff BEFORE DELETE ON codex_observed_completions
        BEGIN SELECT RAISE(ABORT,'injected handoff failure'); END;",
    )
    .unwrap();
    let error = goal_progress::stage(&db, "job", "T1", 7, "original").unwrap_err();
    assert!(error.to_string().contains("injected handoff failure"));
    assert!(goal_progress::pending(&db).unwrap().is_empty());
    assert!(!queue::list(&db).unwrap()[0].goal_waiting);
    assert!(observed_completion::contains(&db, "thread", "T1").unwrap());
    c.execute_batch("DROP TRIGGER fail_handoff").unwrap();
    drop(c);
    goal_progress::stage(&db, "job", "T1", 7, "original").unwrap();
    assert!(queue::list(&db).unwrap()[0].goal_waiting);
    assert!(!observed_completion::contains(&db, "thread", "T1").unwrap());
    assert_eq!(goal_progress::pending(&db).unwrap()[0].content, "original");
}

#[test]
fn repeated_handoff_preserves_original_and_refuses_stale_turn_or_generation() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    setup(&db);
    for _ in 0..2 {
        goal_progress::stage(&db, "job", "T1", 7, "original").unwrap();
    }
    assert!(goal_progress::stage(&db, "job", "T1", 7, "changed").is_err());
    assert!(goal_progress::stage(&db, "job", "T1", 6, "original").is_err());
    assert!(queue::attach_goal_turn(&db, "thread", "T2", 7).unwrap());
    assert!(goal_progress::stage(&db, "job", "T1", 7, "original").is_err());
    let pending = goal_progress::pending(&db).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "original");
    assert_eq!(queue::list(&db).unwrap()[0].turn_id.as_deref(), Some("T2"));
    assert!(!queue::list(&db).unwrap()[0].goal_waiting);
}
