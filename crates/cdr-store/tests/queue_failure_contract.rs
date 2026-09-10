use cdr_store::queue::{
    NewQueueJob, QueueJobState, begin_attempt, enqueue, list, mark_running,
    record_preflight_failure,
};

fn job(job_id: &str, message_id: i64) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id,
        target_thread_id: "thread-a",
        channel_id: 10,
        owner_user_id: Some(20),
        discord_message_id: Some(message_id),
        app_server_generation: 7,
        prompt: "preserve this prompt",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}

#[test]
fn preflight_failure_increments_pending_attempt_once_and_preserves_identity() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("queue.sqlite");
    enqueue(&db, job("job-a", 99)).unwrap();

    let failed = record_preflight_failure(&db, "job-a", 7, "resume unavailable")
        .unwrap()
        .expect("pending head is updated");

    assert_eq!(failed.state, QueueJobState::Pending);
    assert_eq!(failed.attempt_count, 1);
    assert_eq!(failed.last_error, "resume unavailable");
    assert_eq!(failed.job_id, "job-a");
    assert_eq!(failed.discord_message_id, Some(99));
    assert_eq!(failed.prompt, "preserve this prompt");
    assert!(failed.updated_at > failed.created_at);
    assert_eq!(list(&db).unwrap(), vec![failed]);
}

#[test]
fn preflight_failure_never_mutates_starting_or_running_jobs() {
    for running in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("queue.sqlite");
        enqueue(&db, job("job-a", 99)).unwrap();
        let starting = begin_attempt(&db, "job-a", &[], 7).unwrap();
        let before = if running {
            mark_running(&db, "job-a", "turn-a", 7).unwrap()
        } else {
            starting
        };

        let result = record_preflight_failure(&db, "job-a", 7, "must not overwrite").unwrap();

        assert_eq!(result, None);
        assert_eq!(list(&db).unwrap(), vec![before]);
    }
}
