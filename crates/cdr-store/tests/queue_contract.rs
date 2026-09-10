use cdr_store::StoreError;
use cdr_store::queue::{
    NewQueueJob, QueueJobState, adopt_generation, adopt_target_generation, attach_goal_turn,
    begin_attempt, complete, discard_for_generation, discard_observed, enqueue, flush, list,
    list_filtered, mark_goal_waiting, mark_running, record_start_failure, retract,
};
use rusqlite::Connection;

fn new_job<'a>(job_id: &'a str, message_id: i64, prompt: &'a str) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id,
        target_thread_id: "thread-1",
        channel_id: 100,
        owner_user_id: Some(200),
        discord_message_id: Some(message_id),
        app_server_generation: 7,
        prompt,
        queued: true,
        ack_sent: false,
        created_at: 10.0,
    }
}

fn customized_job<'a>(
    job_id: &'a str,
    message_id: i64,
    target: &'a str,
    generation: i64,
    created_at: f64,
) -> NewQueueJob<'a> {
    NewQueueJob {
        target_thread_id: target,
        app_server_generation: generation,
        created_at,
        ..new_job(job_id, message_id, job_id)
    }
}

#[test]
fn q1_duplicate_discord_message_returns_the_durable_first_job() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("queue.sqlite");

    let first =
        enqueue(&path, new_job("job-first", 999, "first")).expect("enqueue first durable job");
    assert!(first.created);
    assert_eq!(first.job.state, QueueJobState::Pending);

    let duplicate =
        enqueue(&path, new_job("job-second", 999, "second")).expect("idempotent duplicate enqueue");
    assert!(!duplicate.created);
    assert_eq!(duplicate.job.job_id, "job-first");
    assert_eq!(duplicate.job.prompt, "first");

    let reopened = list(&path).expect("list durable jobs after reopen");
    assert_eq!(reopened, vec![first.job]);
}

#[test]
fn q2_attempt_running_flush_and_completion_are_durable_and_scoped() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("queue.sqlite");
    enqueue(&path, customized_job("job-a", 1, "thread-a", 7, 1.0)).expect("enqueue job a");
    enqueue(&path, customized_job("job-b", 2, "thread-b", 7, 2.0)).expect("enqueue job b");

    let baseline = vec!["old-turn-1".to_owned(), "old-turn-2".to_owned()];
    let starting = begin_attempt(&path, "job-a", &baseline, 7).expect("begin queue attempt");
    assert_eq!(starting.state, QueueJobState::Starting);
    assert_eq!(starting.attempt_count, 1);
    assert_eq!(starting.baseline_turn_ids, baseline);
    assert_eq!(starting.turn_id, None);

    let running = mark_running(&path, "job-a", "turn-new", 7).expect("mark queue running");
    assert_eq!(running.state, QueueJobState::Running);
    assert_eq!(running.turn_id.as_deref(), Some("turn-new"));
    let wrong_generation =
        mark_running(&path, "job-a", "wrong", 8).expect_err("wrong generation cannot update job");
    assert!(matches!(wrong_generation, StoreError::QueueJobNotFound(_)));

    let flushed = flush(&path, "thread-a", 7).expect("flush target queue");
    assert_eq!(flushed, vec![running]);
    assert_eq!(
        list_filtered(&path, Some("thread-a"), None).expect("list a"),
        vec![]
    );
    assert_eq!(list(&path).expect("other job survives").len(), 1);
    assert!(complete(&path, "job-b").expect("complete other job"));
    assert!(!complete(&path, "job-b").expect("second completion is idempotent"));
    assert!(list(&path).expect("queue empty").is_empty());
}

#[test]
fn q3_generation_and_retract_operations_preserve_nonmatching_rows() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("queue.sqlite");
    enqueue(&path, customized_job("job-old", 10, "thread-a", 1, 1.0)).expect("enqueue old job");
    enqueue(&path, customized_job("job-new", 11, "thread-a", 2, 2.0)).expect("enqueue new job");

    let adoption = adopt_generation(&path, 7).expect("adopt persisted queue");
    assert_eq!(adoption.adopted_count, 2);
    assert!(
        adoption
            .jobs
            .iter()
            .all(|job| job.app_server_generation == 7)
    );
    let observed = adoption.jobs;

    Connection::open(&path)
        .expect("open queue for race fixture")
        .execute(
            "UPDATE codex_turn_queue SET app_server_generation = 8 WHERE job_id = 'job-new'",
            [],
        )
        .expect("simulate readoption race");
    let removed = discard_observed(&path, &observed).expect("discard observed generation only");
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].job_id, "job-old");
    assert_eq!(
        list(&path).expect("readopted row survives")[0].job_id,
        "job-new"
    );

    enqueue(&path, customized_job("job-stale", 12, "thread-b", 9, 3.0))
        .expect("enqueue stale generation");
    let stale = discard_for_generation(&path, Some(8)).expect("discard stale generations");
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].job_id, "job-stale");

    enqueue(
        &path,
        customized_job("job-retract-1", 13, "thread-r", 8, 4.0),
    )
    .expect("enqueue retract one");
    enqueue(
        &path,
        customized_job("job-retract-2", 14, "thread-r", 8, 5.0),
    )
    .expect("enqueue retract two");
    let retracted = retract(&path, "thread-r", Some(100), Some(200))
        .expect("retract latest matching job")
        .expect("matching job exists");
    assert_eq!(retracted.job_id, "job-retract-2");
    let remaining = list_filtered(&path, Some("thread-r"), Some(8)).expect("list remaining");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].job_id, "job-retract-1");
}

#[test]
fn q3b_target_generation_adoption_does_not_touch_unavailable_siblings() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("queue.sqlite");
    enqueue(&path, customized_job("job-a", 21, "thread-a", 1, 1.0)).unwrap();
    enqueue(&path, customized_job("job-b", 22, "thread-b", 2, 2.0)).unwrap();

    let adoption = adopt_target_generation(&path, "thread-b", 7).unwrap();

    assert_eq!(adoption.adopted_count, 1);
    assert_eq!(adoption.jobs.len(), 1);
    assert_eq!(adoption.jobs[0].job_id, "job-b");
    let jobs = list(&path).unwrap();
    assert_eq!(jobs[0].app_server_generation, 1);
    assert_eq!(jobs[1].app_server_generation, 7);
}

#[test]
fn q4_start_failures_distinguish_safe_retry_from_ambiguous_delivery() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("queue.sqlite");
    enqueue(&path, new_job("job", 50, "prompt")).expect("enqueue");
    let _ = begin_attempt(&path, "job", &[], 7).expect("start attempt");

    let ambiguous =
        record_start_failure(&path, "job", 7, "timeout", true).expect("record ambiguous failure");
    assert_eq!(ambiguous.state, QueueJobState::Starting);
    assert_eq!(ambiguous.last_error, "timeout");

    let retryable = record_start_failure(&path, "job", 7, &"오류".repeat(600), false)
        .expect("record retryable failure");
    assert_eq!(retryable.state, QueueJobState::Pending);
    assert_eq!(retryable.last_error.chars().count(), 1_000);
}

#[test]
fn q5_goal_continuations_rebind_the_same_durable_job() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("queue.sqlite");
    enqueue(&path, new_job("job", 60, "goal prompt")).unwrap();
    begin_attempt(&path, "job", &[], 7).unwrap();
    mark_running(&path, "job", "turn-1", 7).unwrap();

    assert!(mark_goal_waiting(&path, "job", "turn-1", 7).unwrap());
    assert_eq!(list(&path).unwrap()[0].turn_id.as_deref(), Some("turn-1"));
    assert!(list(&path).unwrap()[0].goal_waiting);
    assert!(attach_goal_turn(&path, "thread-1", "turn-2", 7).unwrap());
    let rebound = list(&path).unwrap();
    assert_eq!(rebound[0].turn_id.as_deref(), Some("turn-2"));
    assert!(!rebound[0].goal_waiting);
    assert!(!attach_goal_turn(&path, "thread-1", "turn-3", 7).unwrap());
}
