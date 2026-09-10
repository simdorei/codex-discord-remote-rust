use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{broadcast, watch};
use tokio::time::timeout;

use super::{
    ResidentForwarders, ResidentNotificationEvent, prepare_forwarders, spawn_notification_forwarder,
};
use crate::client::startup_tests::helper_config;
use crate::manager::admission::ResidentState;
use crate::{AppServerClient, Notification};

fn monitored_forwarders(
    client: &AppServerClient,
    state: ResidentState,
    generation: u64,
    generation_rx: watch::Receiver<u64>,
) -> ResidentForwarders {
    let (notifications, _) = broadcast::channel(8);
    let (server_requests, _) = broadcast::channel(8);
    let forwarders = prepare_forwarders(
        client,
        generation,
        notifications,
        server_requests,
        generation_rx,
    )
    .with_death_monitor(client, state, generation);
    assert_eq!(
        forwarders.handles.len(),
        3,
        "death monitor must be owned by the forwarder join set"
    );
    forwarders
}

#[tokio::test]
async fn queued_old_event_is_drained_before_forwarder_exit() {
    let (source, source_rx) = broadcast::channel(8);
    let (target, mut target_rx) = broadcast::channel(8);
    let (generation, generation_rx) = watch::channel(1);
    let (activation, activation_rx) = watch::channel(false);
    let handle =
        spawn_notification_forwarder(source_rx, 1, target.clone(), generation_rx, activation_rx);
    source
        .send(Notification {
            method: "test/boundary".to_owned(),
            params: json!({}),
        })
        .expect("queue old event");
    generation.send(0).expect("seal old generation");
    activation.send(true).expect("activate forwarder");
    handle.await.expect("forwarder task");

    let ResidentNotificationEvent::Notification {
        generation,
        notification,
    } = target_rx.try_recv().expect("drained event")
    else {
        panic!("unexpected notification gap");
    };
    assert_eq!(generation, 1);
    assert_eq!(notification.method, "test/boundary");
    assert!(matches!(
        target_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn pre_activation_close_does_not_propagate_and_sender_drop_joins() {
    let client = AppServerClient::start(helper_config())
        .await
        .expect("start fixture");
    let state = ResidentState::new(client.clone());
    let (_generation, generation_rx) = watch::channel(1);
    let forwarders = monitored_forwarders(&client, state.clone(), 1, generation_rx);

    client.close().await.expect("close before activation");
    let before_join = state.snapshot();
    assert_eq!(before_join.generation, 1);
    assert!(before_join.accepting);
    assert!(!before_join.restart_pending);

    timeout(Duration::from_secs(2), forwarders.join())
        .await
        .expect("activation sender drop must release every forwarder");
    let after_join = state.snapshot();
    assert!(after_join.accepting);
    assert!(!after_join.restart_pending);
}

#[tokio::test]
async fn activated_stale_monitor_joins_on_generation_change_before_old_close() {
    let old_client = AppServerClient::start(helper_config())
        .await
        .expect("start old fixture");
    let replacement = AppServerClient::start(helper_config())
        .await
        .expect("start replacement fixture");
    let state = ResidentState::new(old_client.clone());
    let (generation, generation_rx) = watch::channel(1);
    let forwarders = monitored_forwarders(&old_client, state.clone(), 1, generation_rx);
    forwarders.activate();

    state
        .record_replacement(&replacement, 2)
        .expect("record replacement");
    state
        .install_replacement(&replacement, 2)
        .expect("install replacement");
    generation.send(2).expect("advance generation");
    timeout(Duration::from_secs(2), forwarders.join())
        .await
        .expect("stale monitor must join before the old client closes");
    old_client.close().await.expect("close old fixture");

    let snapshot = state.snapshot();
    assert_eq!(snapshot.generation, 2);
    assert!(snapshot.accepting);
    assert!(!snapshot.restart_pending);
    assert!(
        snapshot
            .client
            .as_ref()
            .is_some_and(|client| Arc::ptr_eq(&client.inner, &replacement.inner))
    );
    replacement
        .close()
        .await
        .expect("close replacement fixture");
}
