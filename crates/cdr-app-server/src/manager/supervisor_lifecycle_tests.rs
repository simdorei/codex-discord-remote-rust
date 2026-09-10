use std::future::ready;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{Notify, mpsc, watch};
use tokio::time::{Instant, advance, timeout};

use super::supervisor::run_restart_supervisor_using;
use crate::AppServerError;

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

async fn join_supervisor(shutdown: &watch::Sender<bool>, supervisor: tokio::task::JoinHandle<()>) {
    shutdown.send(true).expect("stop restart supervisor");
    timeout(TEST_TIMEOUT, supervisor)
        .await
        .expect("restart supervisor did not stop")
        .expect("restart supervisor task failed");
}

// SUP-C2: all duplicate signals for generation one produce one successful
// replacement attempt. A later stale generation-one signal cannot create 3.
#[tokio::test]
async fn duplicate_same_generation_signals_coalesce_to_one_replacement() {
    let (restart, restart_rx) = watch::channel(None);
    let (shutdown, shutdown_rx) = watch::channel(false);
    let (attempts, mut observed_attempts) = mpsc::unbounded_channel();
    let supervisor = tokio::spawn(run_restart_supervisor_using(
        restart_rx,
        shutdown_rx,
        move |generation| {
            attempts
                .send(generation)
                .expect("record replacement attempt");
            ready(Ok::<bool, AppServerError>(true))
        },
    ));

    for _ in 0..16 {
        restart.send_replace(Some(1));
    }
    assert_eq!(
        timeout(TEST_TIMEOUT, observed_attempts.recv())
            .await
            .expect("supervisor did not observe restart signal"),
        Some(1)
    );

    for _ in 0..256 {
        tokio::task::yield_now().await;
    }
    assert!(observed_attempts.try_recv().is_err());

    restart.send_replace(Some(1));
    for _ in 0..256 {
        tokio::task::yield_now().await;
    }
    assert!(
        observed_attempts.try_recv().is_err(),
        "stale generation-one signal started a second replacement"
    );

    join_supervisor(&shutdown, supervisor).await;
}

// SUP-C2/C3: duplicate watch notifications cannot reset, shorten, or extend
// the current generation's exact 250 ms retry deadline.
#[tokio::test(start_paused = true)]
async fn duplicate_signals_preserve_one_attempt_until_backoff_deadline() {
    let (restart, restart_rx) = watch::channel(Some(7));
    let (shutdown, shutdown_rx) = watch::channel(false);
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempted = Arc::new(Notify::new());
    let attempts_for_runner = Arc::clone(&attempts);
    let attempted_for_runner = Arc::clone(&attempted);
    let supervisor = tokio::spawn(run_restart_supervisor_using(
        restart_rx,
        shutdown_rx,
        move |_| {
            attempts_for_runner.fetch_add(1, Ordering::SeqCst);
            attempted_for_runner.notify_one();
            ready(Ok::<bool, AppServerError>(false))
        },
    ));

    timeout(TEST_TIMEOUT, attempted.notified())
        .await
        .expect("first restart attempt did not run");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    for _ in 0..100 {
        restart.send_replace(Some(7));
    }

    advance(Duration::from_millis(249)).await;
    tokio::task::yield_now().await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    advance(Duration::from_millis(1)).await;
    timeout(TEST_TIMEOUT, attempted.notified())
        .await
        .expect("retry did not run at the preserved deadline");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);

    join_supervisor(&shutdown, supervisor).await;
}

// SUP-C4: runtime shutdown wakes an in-progress 250 ms backoff without
// advancing virtual time, then no retry can occur before explicit close.
#[tokio::test(start_paused = true)]
async fn shutdown_wakes_backoff_and_prevents_later_replacement() {
    let (_restart, restart_rx) = watch::channel(Some(1));
    let (shutdown, shutdown_rx) = watch::channel(false);
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempted = Arc::new(Notify::new());
    let attempts_for_runner = Arc::clone(&attempts);
    let attempted_for_runner = Arc::clone(&attempted);
    let supervisor = tokio::spawn(run_restart_supervisor_using(
        restart_rx,
        shutdown_rx,
        move |_| {
            attempts_for_runner.fetch_add(1, Ordering::SeqCst);
            attempted_for_runner.notify_one();
            ready(Ok::<bool, AppServerError>(false))
        },
    ));

    timeout(TEST_TIMEOUT, attempted.notified())
        .await
        .expect("first restart attempt did not run");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let before_shutdown = Instant::now();

    shutdown.send(true).expect("signal runtime shutdown");
    timeout(Duration::from_millis(1), supervisor)
        .await
        .expect("shutdown slept through the restart backoff")
        .expect("restart supervisor task failed");
    assert_eq!(Instant::now(), before_shutdown);

    advance(Duration::from_secs(1)).await;
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "restart attempt happened after shutdown"
    );
}
