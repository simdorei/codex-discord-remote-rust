#[path = "support/queue_backoff.rs"]
mod support;

use std::collections::VecDeque;
use std::sync::Arc;

use cdr_runtime::queue_runner::BackendFailure;
use cdr_store::queue::{QueueJobState, begin_attempt, list, mark_running};
use support::{BackoffBackend, coordinator, enqueue_job};

#[tokio::test]
async fn delayed_fifo_head_skips_its_target_while_a_later_target_progresses() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("queue.sqlite");
    let backend = Arc::new(BackoffBackend::default());
    backend.start_failures.lock().await.insert(
        "thread-a".into(),
        VecDeque::from([BackendFailure::definite("target a failed")]),
    );
    let queue = coordinator(&db, &backend);
    let accepted = queue
        .submit("thread-a", 10, 20, Some(1), "a-first")
        .await
        .unwrap();
    assert_eq!(
        accepted.warning,
        Some(BackendFailure::definite("target a failed"))
    );
    enqueue_job(&db, "a-second", "thread-a", 2, 9_000_000_000.0);
    enqueue_job(&db, "b-first", "thread-b", 3, 3.0);
    let calls_before = backend.snapshot().await;

    let report = queue.recover().await.unwrap();

    assert_eq!(report.started, 1);
    let after = backend.snapshot().await;
    assert_eq!(
        after
            .active
            .iter()
            .filter(|target| *target == "thread-a")
            .count(),
        calls_before
            .active
            .iter()
            .filter(|target| *target == "thread-a")
            .count()
    );
    assert!(!after.resumes[calls_before.resumes.len()..].contains(&"thread-a".into()));
    assert_eq!(
        after.starts.last(),
        Some(&("thread-b".into(), "b-first".into()))
    );
    let jobs = list(&db).unwrap();
    assert_eq!(
        jobs.iter()
            .find(|job| job.prompt == "a-first")
            .unwrap()
            .attempt_count,
        1
    );
    assert_eq!(
        jobs.iter()
            .find(|job| job.job_id == "a-second")
            .unwrap()
            .state,
        QueueJobState::Pending
    );
}

#[tokio::test]
async fn resume_and_read_preflight_failures_are_recorded_once_on_pending_head() {
    for read_failure in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("queue.sqlite");
        let backend = Arc::new(BackoffBackend::default());
        let expected = if read_failure {
            backend.read_failures.lock().await.insert("thread-a".into());
            "read failed for thread-a"
        } else {
            backend
                .resume_failures
                .lock()
                .await
                .insert("thread-a".into());
            "resume failed for thread-a"
        };
        enqueue_job(&db, "head", "thread-a", 10, 1.0);
        enqueue_job(&db, "tail", "thread-a", 11, 2.0);
        let queue = coordinator(&db, &backend);

        let report = queue.recover().await.unwrap();

        assert_eq!(report.unavailable_targets.len(), 1);
        let jobs = list(&db).unwrap();
        assert_eq!(jobs[0].attempt_count, 1);
        assert_eq!(jobs[0].last_error, expected);
        assert_eq!(jobs[1].attempt_count, 0);
        assert_eq!(jobs[1].last_error, "");
    }
}

#[tokio::test]
async fn active_state_is_reconciled_from_read_without_requiring_resume() {
    for starting in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("queue.sqlite");
        enqueue_job(&db, "active", "thread-a", 20, 1.0);
        let begun = begin_attempt(&db, "active", &[], 7).unwrap();
        let before = if starting {
            support::make_due(&db, "active");
            begun
        } else {
            mark_running(&db, "active", "turn-a", 7).unwrap()
        };
        let backend = Arc::new(BackoffBackend::default());
        backend
            .resume_failures
            .lock()
            .await
            .insert("thread-a".into());
        let queue = coordinator(&db, &backend);

        queue.recover().await.unwrap();

        let after = list(&db).unwrap();
        if starting {
            assert_eq!(after[0].state, QueueJobState::Pending);
            assert!(after[0].last_error.contains("before a turn appeared"));
        } else {
            assert_eq!(after, vec![before]);
        }
        assert!(backend.snapshot().await.resumes.is_empty());
        assert!(backend.snapshot().await.starts.is_empty());
    }
}

#[tokio::test]
async fn read_only_starting_requeue_honors_durable_pending_backoff() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("queue.sqlite");
    enqueue_job(&db, "active", "thread-a", 20, 1.0);
    let before = begin_attempt(&db, "active", &[], 7).unwrap();
    support::make_due(&db, "active");
    let backend = Arc::new(BackoffBackend::default());
    backend
        .resume_failures
        .lock()
        .await
        .insert("thread-a".into());
    let queue = coordinator(&db, &backend);

    let first = queue.recover().await.unwrap();
    assert_eq!(first.requeued, 1);
    assert!(first.unavailable_targets.is_empty());
    let calls_after_first = backend.snapshot().await;

    let immediate_retry = queue.recover().await.unwrap();
    let calls_after_retry = backend.snapshot().await;

    assert!(immediate_retry.unavailable_targets.is_empty());
    assert_eq!(calls_after_retry.reads, calls_after_first.reads);
    assert_eq!(calls_after_retry.resumes, calls_after_first.resumes);
    let after = list(&db).unwrap();
    assert_eq!(after[0].state, QueueJobState::Pending);
    assert_ne!(after, vec![before]);
}
