use cdr_store::{
    queue::{self, NewQueueJob},
    schema::open_initialized,
};
use std::sync::{Arc, Barrier};

const CONFLICT: &str = "app-server returned error -32600 for thread/resume: thread original already has an active writer";

fn job(id: &str) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id: id,
        target_thread_id: "original",
        channel_id: 42,
        owner_user_id: Some(3),
        discord_message_id: Some(101),
        app_server_generation: 1,
        prompt: "never started",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}

#[test]
fn repeated_writer_preflight_failures_can_be_cancelled_without_replay() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    queue::enqueue(&db, job("job")).unwrap();
    for _ in 0..37 {
        queue::record_preflight_failure(&db, "job", 1, CONFLICT).unwrap();
    }
    let before = queue::list(&db).unwrap().remove(0);
    assert_eq!(before.attempt_count, 37);
    assert_eq!(before.execution_generation, None);
    assert_eq!(
        queue::cancel_latest_pending(&db, "original", 42, 3, 12.0)
            .unwrap()
            .as_deref(),
        Some("job")
    );
    assert!(queue::list(&db).unwrap().is_empty());
    assert!(
        queue::try_begin_attempt(&db, "job", &[], 1)
            .unwrap()
            .is_none()
    );
    assert!(queue::enqueue(&db, job("replacement-with-same-event")).is_err());
}

#[test]
fn uncertain_execution_and_other_errors_remain_uncancellable() {
    for (error, change) in [
        (
            CONFLICT,
            "UPDATE codex_turn_queue SET execution_generation=1",
        ),
        (
            CONFLICT,
            "UPDATE codex_turn_queue SET turn_observation_generation=1",
        ),
        (CONFLICT, "UPDATE codex_turn_queue SET goal_waiting=1"),
        (
            CONFLICT,
            "UPDATE codex_turn_queue SET baseline_turn_ids='[\"prior\"]'",
        ),
        ("request timed out after write", "SELECT 1"),
        (
            "app-server returned error -32600 for thread/resume: thread other already has an active writer",
            "SELECT 1",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        queue::enqueue(&db, job("job")).unwrap();
        queue::record_preflight_failure(&db, "job", 1, error).unwrap();
        open_initialized(&db)
            .unwrap()
            .execute_batch(change)
            .unwrap();
        assert!(
            queue::cancel_latest_pending(&db, "original", 42, 3, 12.0).is_err(),
            "{error} {change}"
        );
        assert_eq!(queue::list(&db).unwrap().len(), 1);
    }
}

#[test]
fn preflight_cancellation_still_requires_original_sender_and_channel() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    queue::enqueue(&db, job("job")).unwrap();
    queue::record_preflight_failure(&db, "job", 1, CONFLICT).unwrap();
    for (thread, channel, user) in [("other", 42, 3), ("original", 43, 3), ("original", 42, 4)] {
        assert!(
            queue::cancel_latest_pending(&db, thread, channel, user, 12.0)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(queue::list(&db).unwrap().len(), 1);
}

#[test]
fn claim_and_writer_conflict_cancellation_cannot_both_win() {
    for _ in 0..8 {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        queue::enqueue(&db, job("job")).unwrap();
        queue::record_preflight_failure(&db, "job", 1, CONFLICT).unwrap();
        let gate = Arc::new(Barrier::new(2));
        let other_gate = gate.clone();
        let other_db = db.clone();
        let claim = std::thread::spawn(move || {
            other_gate.wait();
            queue::try_begin_attempt(&other_db, "job", &[], 1).unwrap()
        });
        gate.wait();
        let cancelled = queue::cancel_latest_pending(&db, "original", 42, 3, 12.0);
        let claimed = claim.join().unwrap();
        assert_ne!(claimed.is_some(), cancelled.is_ok_and(|r| r.is_some()));
    }
}
