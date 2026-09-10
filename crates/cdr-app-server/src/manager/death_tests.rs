use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;

use super::ResidentAppServer;
use super::admission::ResidentState;
use crate::client::startup_tests::helper_config;
use crate::{AppServerClient, AppServerError};

async fn wait_for_death_propagation(server: &ResidentAppServer) {
    timeout(Duration::from_secs(2), async {
        loop {
            let state = server.state.snapshot();
            if !state.accepting && state.restart_pending {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("resident death propagation timeout");
}

async fn assert_generation_two_request_succeeds(server: &ResidentAppServer) {
    assert_eq!(
        server
            .request(
                "test/echo",
                json!({"generation": 2}),
                Duration::from_secs(1),
                Some(2),
            )
            .await
            .expect("generation two request"),
        json!({})
    );
}

#[tokio::test]
async fn quiescent_current_death_closes_admission_and_restarts_next_generation() {
    let server = ResidentAppServer::start(helper_config())
        .await
        .expect("start resident fixture");
    let current = server.state.current_client().expect("current client");

    current.close().await.expect("close current client");
    wait_for_death_propagation(&server).await;

    let state = server.state.snapshot();
    assert_eq!(state.generation, 1);
    assert!(!state.accepting);
    assert!(state.restart_pending);
    assert!(matches!(
        server.state.admit_request(Some(1)),
        Err(AppServerError::Closed)
    ));
    assert!(server.restart_if_quiescent().await.expect("restart"));
    assert_eq!(server.generation(), 2);
    assert_generation_two_request_succeeds(&server).await;
    server.close().await.expect("close resident fixture");
}

#[tokio::test]
async fn installed_replacement_ignores_delayed_old_generation_death() {
    let server = ResidentAppServer::start(helper_config())
        .await
        .expect("start resident fixture");
    let old_client = server
        .state
        .current_client()
        .expect("generation one client");
    assert!(server.force_restart_if_quiescent().await.expect("restart"));
    let installed = server
        .state
        .current_client()
        .expect("generation two client");

    server.state.mark_current_closed(&old_client, 1);

    let state = server.state.snapshot();
    assert_eq!(state.generation, 2);
    assert!(state.accepting);
    assert!(!state.restart_pending);
    assert!(
        state
            .client
            .as_ref()
            .is_some_and(|client| Arc::ptr_eq(&client.inner, &installed.inner))
    );
    assert_generation_two_request_succeeds(&server).await;
    server.close().await.expect("close resident fixture");
}

#[tokio::test]
async fn installed_replacement_monitor_propagates_generation_two_death() {
    let server = ResidentAppServer::start(helper_config())
        .await
        .expect("start resident fixture");
    assert!(server.force_restart_if_quiescent().await.expect("restart"));
    let installed = server
        .state
        .current_client()
        .expect("generation two client");

    installed
        .close()
        .await
        .expect("close generation two client");
    wait_for_death_propagation(&server).await;

    let snapshot = server.state.snapshot();
    assert_eq!(snapshot.generation, 2);
    assert!(!snapshot.accepting);
    assert!(snapshot.restart_pending);
    assert!(matches!(
        server.state.admit_request(Some(2)),
        Err(AppServerError::Closed)
    ));
    server.close().await.expect("close resident fixture");
}

#[tokio::test]
async fn stale_death_callbacks_do_not_change_current_generation() {
    let current = AppServerClient::start(helper_config())
        .await
        .expect("start current fixture");
    let old_client = AppServerClient::start(helper_config())
        .await
        .expect("start stale fixture");
    let state = ResidentState::new(current.clone());

    state.mark_current_closed(&old_client, 1);
    state.mark_current_closed(&current, 2);

    let snapshot = state.snapshot();
    assert!(snapshot.accepting);
    assert!(!snapshot.restart_pending);
    old_client.close().await.expect("close stale fixture");
    current.close().await.expect("close current fixture");
}

#[tokio::test]
async fn terminal_state_ignores_matching_death_callback() {
    let current = AppServerClient::start(helper_config())
        .await
        .expect("start current fixture");
    let state = ResidentState::new(current.clone());
    let _plan = state.prepare_close();

    state.mark_current_closed(&current, 1);

    let snapshot = state.snapshot();
    assert!(!snapshot.restart_pending);
    current.close().await.expect("close terminal fixture");
    state
        .finish_current_close(&current)
        .expect("finish terminal fixture");
}
