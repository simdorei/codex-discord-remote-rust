use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{Notify, watch};
use tokio::time::{advance, timeout};

use super::ResidentAppServer;
use super::admission::ResidentState;
use super::supervisor::run_restart_supervisor_using;
use crate::AppServerClient;
use crate::client::startup_tests::helper_config;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

fn assert_watch_update(receiver: &mut watch::Receiver<Option<u64>>, expected: Option<u64>) {
    assert!(
        receiver
            .has_changed()
            .expect("restart watch sender dropped"),
        "restart transition did not publish a watch update"
    );
    assert_eq!(*receiver.borrow_and_update(), expected);
}

async fn stop_supervisor(shutdown: &watch::Sender<bool>, supervisor: tokio::task::JoinHandle<()>) {
    shutdown.send(true).expect("stop restart supervisor");
    timeout(TEST_TIMEOUT, supervisor)
        .await
        .expect("restart supervisor did not stop")
        .expect("restart supervisor task failed");
}

async fn generation_two_state() -> (ResidentState, AppServerClient, AppServerClient) {
    let old = AppServerClient::start(helper_config())
        .await
        .expect("generation one fixture");
    let current = AppServerClient::start(helper_config())
        .await
        .expect("generation two fixture");
    let state = ResidentState::new(old.clone());
    state.request_restart();
    state
        .record_replacement(&current, 2)
        .expect("record generation two");
    state
        .install_replacement(&current, 2)
        .expect("install generation two");
    (state, old, current)
}

// SUP-C2: an explicitly requested restart must report success when a supervisor
// already owns the same generation's work, without turning that request into 3.
#[tokio::test]
async fn force_restart_racing_enqueued_supervisor_reports_one_generation_success() {
    let server = Arc::new(
        ResidentAppServer::start(helper_config())
            .await
            .expect("start resident fixture"),
    );
    let restart_guard = server.restart_lock.lock().await;
    server.state.request_restart();
    let restart_rx = server.state.subscribe_restart_pending();
    let (shutdown, shutdown_rx) = watch::channel(false);
    let entered = Arc::new(Notify::new());
    let entered_for_attempt = Arc::clone(&entered);
    let server_for_attempt = Arc::clone(&server);
    let supervisor = tokio::spawn(run_restart_supervisor_using(
        restart_rx,
        shutdown_rx,
        move |generation| {
            let server = Arc::clone(&server_for_attempt);
            let entered = Arc::clone(&entered_for_attempt);
            async move {
                entered.notify_one();
                server.restart_generation_if_quiescent(generation).await
            }
        },
    ));
    timeout(TEST_TIMEOUT, entered.notified())
        .await
        .expect("supervisor did not enqueue generation one");

    let force_entered = Arc::new(Notify::new());
    let force_entered_by_task = Arc::clone(&force_entered);
    let server_for_force = Arc::clone(&server);
    let forced = tokio::spawn(async move {
        force_entered_by_task.notify_one();
        server_for_force.force_restart_if_quiescent().await
    });
    timeout(TEST_TIMEOUT, force_entered.notified())
        .await
        .expect("explicit force restart did not enqueue");
    drop(restart_guard);
    let forced = timeout(TEST_TIMEOUT, forced)
        .await
        .expect("explicit force restart timed out")
        .expect("explicit force restart task failed");
    stop_supervisor(&shutdown, supervisor).await;

    let snapshot = server.lifecycle_snapshot().await;
    server.close().await.expect("close resident fixture");
    assert!(forced.expect("explicit force restart failed"));
    assert_eq!(snapshot.generation, 2, "same restart was applied twice");
    assert!(snapshot.healthy);
    assert!(!snapshot.restart_pending);
}

// SUP-C6/C8: a delayed generation-one supervisor attempt is a no-op even when
// generation two owns a distinct generation-three cleanup obligation.
#[tokio::test]
async fn stale_generation_attempt_preserves_newer_cleanup_and_admissions() {
    let server = ResidentAppServer::start(helper_config())
        .await
        .expect("start resident fixture");
    assert!(
        server
            .force_restart_if_quiescent()
            .await
            .expect("install 2")
    );
    let current = server
        .state
        .current_client()
        .expect("generation two client");
    let cleanup = AppServerClient::start(helper_config())
        .await
        .expect("generation three cleanup fixture");
    server
        .state
        .record_replacement(&cleanup, 3)
        .expect("record newer cleanup debt");
    server.state.request_restart();

    assert!(
        server
            .restart_generation_if_quiescent(1)
            .await
            .expect("stale attempt should settle")
    );
    let snapshot = server.state.snapshot();
    let preserved = server
        .state
        .replacement_cleanup()
        .expect("newer cleanup debt was reconciled by stale attempt");
    assert_eq!(snapshot.generation, 2);
    assert!(snapshot.accepting);
    assert!(snapshot.restart_pending);
    assert!(
        snapshot
            .client
            .as_ref()
            .is_some_and(|client| { Arc::ptr_eq(&client.inner, &current.inner) })
    );
    assert_eq!(preserved.generation, 3);
    assert!(Arc::ptr_eq(&preserved.client.inner, &cleanup.inner));
    assert!(cleanup.lifecycle_snapshot().closed_reason.is_none());
    assert_eq!(
        server
            .request(
                "test/echo",
                json!({"generation": 2}),
                Duration::from_secs(1),
                Some(2),
            )
            .await
            .expect("generation two admission was sealed"),
        json!({})
    );
    server.close().await.expect("close resident fixture");
}

// SUP-C4: terminal close retracts any pending watch value and permanently
// ignores later restart requests, so a subsequently started supervisor is idle.
#[tokio::test]
async fn terminal_close_clears_watch_and_prevents_respawn() {
    let server = Arc::new(
        ResidentAppServer::start(helper_config())
            .await
            .expect("start resident fixture"),
    );
    let mut restart = server.state.subscribe_restart_pending();
    server.state.request_restart();
    assert_watch_update(&mut restart, Some(1));
    server.close().await.expect("terminal close");
    assert_watch_update(&mut restart, None);

    server.state.request_restart();
    assert_eq!(*restart.borrow_and_update(), None);
    tokio::time::pause();
    let (shutdown, shutdown_rx) = watch::channel(false);
    let supervisor = tokio::spawn(Arc::clone(&server).run_restart_supervisor(shutdown_rx));
    advance(Duration::from_secs(30)).await;
    tokio::task::yield_now().await;
    assert_eq!(server.generation(), 1, "terminal server respawned");
    assert!(!server.state.snapshot().restart_pending);
    stop_supervisor(&shutdown, supervisor).await;
}

// SUP-C1/C3/C4: every restart-producing state transition publishes a noninitial
// current generation, and successful replacement installation retracts it.
#[tokio::test]
async fn restart_transitions_publish_generation_and_success_publishes_none() {
    let (requested_state, requested_old, requested) = generation_two_state().await;
    let mut requested_watch = requested_state.subscribe_restart_pending();
    requested_state.request_restart();
    assert_watch_update(&mut requested_watch, Some(2));

    let (timeout_state, timeout_old, timed_out) = generation_two_state().await;
    let mut timeout_watch = timeout_state.subscribe_restart_pending();
    timeout_state.mark_timeout(2);
    assert_watch_update(&mut timeout_watch, Some(2));

    let (death_state, death_old, died) = generation_two_state().await;
    let mut death_watch = death_state.subscribe_restart_pending();
    death_state.mark_current_closed(&died, 2);
    assert_watch_update(&mut death_watch, Some(2));

    let replacement = AppServerClient::start(helper_config())
        .await
        .expect("replacement fixture");
    requested_state
        .record_replacement(&replacement, 3)
        .expect("record replacement");
    requested_state
        .install_replacement(&replacement, 3)
        .expect("install replacement");
    assert_watch_update(&mut requested_watch, None);

    requested_old
        .close()
        .await
        .expect("close old request fixture");
    requested.close().await.expect("close original fixture");
    replacement
        .close()
        .await
        .expect("close replacement fixture");
    timeout_old
        .close()
        .await
        .expect("close old timeout fixture");
    timed_out.close().await.expect("close timeout fixture");
    death_old.close().await.expect("close old death fixture");
    died.close().await.expect("close death fixture");
}
