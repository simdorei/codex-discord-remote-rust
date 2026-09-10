use std::collections::HashMap;
use std::future::ready;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::{advance, timeout};

use super::ResidentAppServer;
use super::supervisor::run_restart_supervisor_using;
use crate::client::startup_tests::helper_config;
use crate::{AppServerError, Notification, RequestId, ServerRequest, ServerRequestOccurrence};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const SCHEDULE_TIMEOUT: Duration = Duration::from_millis(1);
type ObservedAttempt = (u64, Result<bool, String>);

fn observed_supervisor(
    server: Arc<ResidentAppServer>,
    shutdown: watch::Receiver<bool>,
) -> (
    tokio::task::JoinHandle<()>,
    mpsc::UnboundedReceiver<ObservedAttempt>,
) {
    let restart = server.state.subscribe_restart_pending();
    let (attempts, observed) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_restart_supervisor_using(
        restart,
        shutdown,
        move |generation| {
            let server = Arc::clone(&server);
            let attempts = attempts.clone();
            async move {
                let result = server.restart_generation_if_quiescent(generation).await;
                let observed = result.as_ref().copied().map_err(ToString::to_string);
                attempts
                    .send((generation, observed))
                    .expect("record attempt");
                result
            }
        },
    ));
    (task, observed)
}

async fn recv_attempt(receiver: &mut mpsc::UnboundedReceiver<ObservedAttempt>) -> ObservedAttempt {
    timeout(TEST_TIMEOUT, receiver.recv())
        .await
        .expect("restart attempt timed out")
        .expect("restart attempt channel closed")
}

async fn stop_supervisor(shutdown: &watch::Sender<bool>, task: tokio::task::JoinHandle<()>) {
    shutdown.send(true).expect("signal supervisor shutdown");
    timeout(TEST_TIMEOUT, task)
        .await
        .expect("supervisor did not stop")
        .expect("supervisor task failed");
}

async fn recv_scheduled(receiver: &mut mpsc::UnboundedReceiver<(u64, usize)>) -> (u64, usize) {
    timeout(SCHEDULE_TIMEOUT, receiver.recv())
        .await
        .expect("scheduled attempt did not run")
        .expect("scheduled attempt channel closed")
}

// SUP-C5: an admitted operation makes the automatic decision defer without
// replacing the process; dropping its permit permits the scheduled retry.
#[tokio::test]
async fn admitted_operation_blocks_automatic_replacement_until_settled() {
    let server = Arc::new(
        ResidentAppServer::start(helper_config())
            .await
            .expect("start resident fixture"),
    );
    let admission = server.state.admit_request(Some(1)).expect("admit RPC");
    let original_pid = server.lifecycle_snapshot().await.process_id;
    server.state.request_restart();
    let (shutdown, shutdown_rx) = watch::channel(false);
    let (supervisor, mut attempts) = observed_supervisor(Arc::clone(&server), shutdown_rx);

    assert_eq!(recv_attempt(&mut attempts).await, (1, Ok(false)));
    let held = server.lifecycle_snapshot().await;
    assert_eq!(held.generation, 1);
    assert_eq!(held.process_id, original_pid);
    assert!(held.healthy && held.restart_pending);

    drop(admission);
    assert_eq!(recv_attempt(&mut attempts).await, (1, Ok(true)));
    assert_eq!(server.generation(), 2);
    stop_supervisor(&shutdown, supervisor).await;
    server.close().await.expect("close resident fixture");
}

// SUP-C5: closed admission remains fail-closed with either an active turn or an
// unsettled request; clearing that one blocker allows automatic recovery.
#[tokio::test]
async fn active_turn_or_unsettled_request_independently_blocks_then_recovers() {
    for active_turn in [true, false] {
        let server = Arc::new(
            ResidentAppServer::start(helper_config())
                .await
                .expect("start resident fixture"),
        );
        let client = server
            .state
            .current_client()
            .expect("generation one client");
        let request = ServerRequest {
            id: RequestId::String("approval-1".to_owned()),
            occurrence: ServerRequestOccurrence::from_bytes(1_u128.to_be_bytes()),
            method: "item/commandExecution/requestApproval".to_owned(),
            params: json!({"threadId":"thread-a","turnId":"turn-a"}),
        };
        {
            let mut state = client.inner.state.lock().expect("runtime state lock");
            if active_turn {
                state.record_notification(Notification {
                    method: "turn/started".to_owned(),
                    params: json!({"threadId":"thread-a","turn":{"id":"turn-a"}}),
                });
            } else {
                state
                    .record_server_request(request.clone())
                    .expect("record unsettled request");
            }
        }
        assert_eq!(client.active_turn_id("thread-a").is_some(), active_turn);
        assert_eq!(client.has_unsettled_server_requests(), !active_turn);
        server.state.mark_current_closed(&client, 1);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (supervisor, mut attempts) = observed_supervisor(Arc::clone(&server), shutdown_rx);

        assert_eq!(recv_attempt(&mut attempts).await, (1, Ok(false)));
        assert_eq!(server.generation(), 1);
        assert!(matches!(
            server.state.admit_request(Some(1)),
            Err(AppServerError::Closed)
        ));
        {
            let mut state = client.inner.state.lock().expect("runtime state lock");
            if active_turn {
                state.record_notification(Notification {
                    method: "turn/completed".to_owned(),
                    params: json!({"threadId":"thread-a","turn":{"id":"turn-a"}}),
                });
            } else {
                state
                    .begin_server_response(&request.id, request.occurrence)
                    .expect("claim exact request");
                state
                    .resolve_server_request(&request.id, request.occurrence)
                    .expect("resolve exact request");
            }
        }
        assert!(client.active_turn_id("thread-a").is_none());
        assert!(!client.has_unsettled_server_requests());
        assert_eq!(recv_attempt(&mut attempts).await, (1, Ok(true)));
        assert_eq!(server.generation(), 2);
        stop_supervisor(&shutdown, supervisor).await;
        server.close().await.expect("close resident fixture");
    }
}

// SUP-C4: shutdown does not cancel an active attempt. It joins after that
// attempt resolves and must not start a second attempt.
#[tokio::test(start_paused = true)]
async fn shutdown_waits_for_blocked_attempt_then_exits_once() {
    let (_restart, restart_rx) = watch::channel(Some(1));
    let (shutdown, shutdown_rx) = watch::channel(false);
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let attempts = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn(run_restart_supervisor_using(restart_rx, shutdown_rx, {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        let attempts = Arc::clone(&attempts);
        move |_| {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                entered.notify_one();
                release.notified().await;
                Ok::<bool, AppServerError>(true)
            }
        }
    }));
    timeout(TEST_TIMEOUT, entered.notified())
        .await
        .expect("attempt did not enter");
    shutdown.send(true).expect("signal shutdown");
    advance(Duration::from_secs(30)).await;
    assert!(!task.is_finished(), "shutdown cancelled active attempt");
    release.notify_one();
    timeout(TEST_TIMEOUT, task)
        .await
        .expect("supervisor did not join")
        .expect("supervisor task failed");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

// SUP-C3: generation-one settlement resets history, so generation two retries
// after 250 ms rather than inheriting the preceding 500 ms delay.
#[tokio::test(start_paused = true)]
async fn successful_settlement_resets_new_generation_retry_to_250ms() {
    let (restart, restart_rx) = watch::channel(Some(1));
    let (shutdown, shutdown_rx) = watch::channel(false);
    let counts = Arc::new(Mutex::new(HashMap::<u64, usize>::new()));
    let (attempts, mut observed) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_restart_supervisor_using(
        restart_rx,
        shutdown_rx,
        move |generation| {
            let ordinal = {
                let mut counts = counts.lock().expect("attempt counts lock");
                let count = counts.entry(generation).or_default();
                *count += 1;
                *count
            };
            attempts
                .send((generation, ordinal))
                .expect("record attempt");
            ready(Ok::<bool, AppServerError>(generation == 1 && ordinal == 2))
        },
    ));
    assert_eq!(recv_scheduled(&mut observed).await, (1, 1));
    advance(Duration::from_millis(250)).await;
    assert_eq!(recv_scheduled(&mut observed).await, (1, 2));
    restart.send_replace(Some(2));
    assert_eq!(recv_scheduled(&mut observed).await, (2, 1));
    advance(Duration::from_millis(249)).await;
    tokio::task::yield_now().await;
    assert!(observed.try_recv().is_err());
    advance(Duration::from_millis(1)).await;
    assert_eq!(recv_scheduled(&mut observed).await, (2, 2));
    stop_supervisor(&shutdown, task).await;
}
