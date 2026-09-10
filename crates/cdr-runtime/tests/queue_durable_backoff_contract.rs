#[path = "support/queue_backoff.rs"]
mod support;

use std::collections::VecDeque;
use std::sync::Arc;

use cdr_runtime::queue_runner::{BackendFailure, Submission};
use cdr_store::delivery::list_pending;
use cdr_store::queue::{QueueJobState, list};
use support::{BackoffBackend, coordinator, make_due};

#[tokio::test]
async fn duplicate_source_message_replays_exact_error_without_backend_redrive() {
    for (failure, expected_state) in [
        (
            BackendFailure::definite("transport failed: exact"),
            QueueJobState::Pending,
        ),
        (
            BackendFailure::ambiguous("transport outcome unknown: exact"),
            QueueJobState::Starting,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("queue.sqlite");
        let backend = Arc::new(BackoffBackend::default());
        backend
            .start_failures
            .lock()
            .await
            .insert("thread-a".into(), VecDeque::from([failure.clone()]));
        let queue = coordinator(&db, &backend);

        let first = queue
            .submit("thread-a", 10, 20, Some(77), "preserve me")
            .await
            .unwrap();
        let original_job_id = list(&db).unwrap()[0].job_id.clone();
        let calls_after_first = backend.snapshot().await;
        let duplicate = queue
            .submit("thread-a", 10, 20, Some(77), "different duplicate body")
            .await
            .unwrap();

        assert_accepted_warning(&first, &failure);
        assert_eq!(duplicate, first);
        assert_eq!(backend.snapshot().await, calls_after_first);
        let jobs = list(&db).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, original_job_id);
        assert_eq!(jobs[0].prompt, "preserve me");
        assert_eq!(jobs[0].attempt_count, 1);
        assert_eq!(jobs[0].state, expected_state);
        assert!(list_pending(&db).unwrap().is_empty());
    }
}

#[tokio::test]
async fn durable_preflight_failure_is_accepted_with_its_exact_warning() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("queue.sqlite");
    let backend = Arc::new(BackoffBackend::default());
    backend
        .resume_failures
        .lock()
        .await
        .insert("thread-a".into());
    let queue = coordinator(&db, &backend);

    let first = queue
        .submit("thread-a", 10, 20, Some(79), "preserve preflight")
        .await
        .unwrap();
    let calls_after_first = backend.snapshot().await;
    let duplicate = queue
        .submit("thread-a", 10, 20, Some(79), "different duplicate body")
        .await
        .unwrap();

    assert_accepted_warning(
        &first,
        &BackendFailure::definite("resume failed for thread-a"),
    );
    assert_eq!(duplicate, first);
    assert_eq!(backend.snapshot().await, calls_after_first);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, QueueJobState::Pending);
    assert_eq!(jobs[0].attempt_count, 1);
}

#[tokio::test]
async fn invalid_pre_enqueue_identifier_remains_an_error_without_a_job() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("queue.sqlite");
    let backend = Arc::new(BackoffBackend::default());
    let queue = coordinator(&db, &backend);

    let error = queue
        .submit("thread-a", u64::MAX, 20, Some(80), "reject me")
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        cdr_runtime::queue_runner::QueueRunnerError::IntegerRange
    ));
    assert!(list(&db).unwrap().is_empty());
    assert!(backend.snapshot().await.starts.is_empty());
}

#[tokio::test]
async fn deferred_ticks_make_no_calls_then_due_retry_increments_only_once() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("queue.sqlite");
    let backend = Arc::new(BackoffBackend::default());
    backend.start_failures.lock().await.insert(
        "thread-a".into(),
        VecDeque::from([BackendFailure::definite("first failure")]),
    );
    let first = coordinator(&db, &backend);
    first
        .submit("thread-a", 10, 20, Some(88), "retry me")
        .await
        .unwrap();
    let before_ticks = backend.snapshot().await;

    first.recover().await.unwrap();
    let reconstructed = coordinator(&db, &backend);
    reconstructed.recover().await.unwrap();

    assert_eq!(backend.snapshot().await, before_ticks);
    assert_eq!(list(&db).unwrap()[0].attempt_count, 1);
    make_due(&db, &list(&db).unwrap()[0].job_id);
    let report = reconstructed.recover().await.unwrap();
    let job = &list(&db).unwrap()[0];
    assert_eq!(report.started, 1);
    assert_eq!(job.attempt_count, 2);
    assert_eq!(job.state, QueueJobState::Running);
}

fn assert_accepted_warning(submission: &Submission, expected: &BackendFailure) {
    assert!(submission.queued);
    assert_eq!(submission.turn_id, None);
    assert_eq!(submission.warning.as_ref(), Some(expected));
}
