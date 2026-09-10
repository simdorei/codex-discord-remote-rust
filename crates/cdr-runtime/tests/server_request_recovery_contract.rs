use std::convert::Infallible;
use std::future::pending;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use cdr_runtime::server_request_worker::recovery::{
    GenerationMismatch, RetrySchedule, SnapshotError, attempt_all, cancel_on_shutdown,
    consistent_snapshot, validate_generation,
};
use tokio::sync::watch;
use tokio::time::{Duration, Instant};

#[tokio::test]
async fn failed_request_does_not_starve_later_pending_requests() {
    let mut attempts = Vec::new();

    let error = attempt_all(
        &mut attempts,
        ["first", "middle", "last"],
        |recorded, request_id| {
            Box::pin(async move {
                recorded.push(request_id);
                match request_id {
                    "first" => Err("first request Discord delivery failed"),
                    "last" => Err("last request Discord delivery failed"),
                    _ => Ok(()),
                }
            })
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error, "first request Discord delivery failed");
    assert_eq!(attempts, ["first", "middle", "last"]);
}

#[tokio::test]
async fn generation_change_retries_the_pending_snapshot() {
    let generation = AtomicU64::new(7);
    let fetches = AtomicUsize::new(0);

    let (snapshot_generation, requests) = consistent_snapshot(
        3,
        || generation.load(Ordering::SeqCst),
        || {
            let fetch = fetches.fetch_add(1, Ordering::SeqCst);
            if fetch == 0 {
                generation.store(8, Ordering::SeqCst);
            }
            async move {
                Ok::<_, Infallible>(if fetch == 0 {
                    vec!["stale request"]
                } else {
                    vec!["current request"]
                })
            }
        },
    )
    .await
    .expect("infallible snapshot");

    assert_eq!(snapshot_generation, 8);
    assert_eq!(requests, ["current request"]);
    assert_eq!(fetches.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn persistent_generation_churn_returns_a_visible_bounded_error() {
    let generation = AtomicU64::new(7);
    let fetches = AtomicUsize::new(0);

    let error = consistent_snapshot(
        3,
        || generation.load(Ordering::SeqCst),
        || {
            fetches.fetch_add(1, Ordering::SeqCst);
            generation.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>(Vec::<&str>::new()) }
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error, SnapshotError::Unstable { attempts: 3 });
    assert_eq!(fetches.load(Ordering::SeqCst), 3);
}

#[test]
fn failed_pending_prompts_are_retried_within_a_bounded_interval() {
    let now = Instant::now();
    let mut retries = RetrySchedule::default();

    for expected in [1, 2, 4, 8, 16, 30, 30] {
        retries.schedule_failure(now);
        assert_eq!(
            retries.deadline().expect("retry") - now,
            Duration::from_secs(expected)
        );
        retries.begin_retry();
    }
    retries.clear();
    assert_eq!(retries.deadline(), None);
    retries.schedule_failure(now);
    assert_eq!(
        retries.deadline().expect("reset retry") - now,
        Duration::from_secs(1)
    );
}

#[tokio::test]
async fn shutdown_cancels_a_pending_recovery_attempt() {
    let (sender, mut shutdown) = watch::channel(false);
    let signal = async move {
        tokio::task::yield_now().await;
        sender.send(true).expect("signal shutdown");
    };

    let (result, ()) = tokio::join!(cancel_on_shutdown(&mut shutdown, pending::<()>()), signal);
    assert_eq!(result, None);
}

#[test]
fn stale_buffered_request_generation_is_rejected() {
    assert_eq!(validate_generation(8, 8), Ok(()));
    assert_eq!(
        validate_generation(7, 8),
        Err(GenerationMismatch {
            expected: 7,
            actual: 8,
        })
    );
}
