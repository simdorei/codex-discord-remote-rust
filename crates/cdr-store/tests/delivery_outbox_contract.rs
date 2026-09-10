use cdr_store::delivery::{complete, list_pending, record_failure, stage_queue_completion};
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, list, mark_running};

fn running_job(path: &std::path::Path) {
    enqueue(
        path,
        NewQueueJob {
            job_id: "job-a",
            target_thread_id: "thread-a",
            channel_id: 100,
            owner_user_id: Some(200),
            discord_message_id: Some(300),
            app_server_generation: 7,
            prompt: "prompt",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_attempt(path, "job-a", &[], 7).unwrap();
    mark_running(path, "job-a", "turn-a", 7).unwrap();
}

#[test]
fn staging_is_atomic_and_preserves_the_delivery_after_queue_removal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    running_job(&path);

    let delivery = stage_queue_completion(&path, "job-a", "final answer", 2.0).unwrap();
    assert_eq!(delivery.delivery_id, "job-a");
    assert_eq!(delivery.turn_id, "turn-a");
    assert_eq!(delivery.channel_id, 100);
    assert_eq!(delivery.content, "final answer");
    assert!(list(&path).unwrap().is_empty());
    assert_eq!(list_pending(&path).unwrap(), vec![delivery]);
}

#[test]
fn failed_delivery_is_durable_bounded_and_completion_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    running_job(&path);
    stage_queue_completion(&path, "job-a", "final answer", 2.0).unwrap();

    let failed = record_failure(&path, "job-a", &"오류".repeat(600), 3.0).unwrap();
    assert_eq!(failed.attempt_count, 1);
    assert_eq!(failed.last_error.chars().count(), 1_000);
    assert_eq!(list_pending(&path).unwrap(), vec![failed]);
    assert!(complete(&path, "job-a").unwrap());
    assert!(!complete(&path, "job-a").unwrap());
    assert!(list_pending(&path).unwrap().is_empty());
}
