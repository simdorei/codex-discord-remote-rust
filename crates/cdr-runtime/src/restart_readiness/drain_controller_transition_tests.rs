use std::fs;
use std::sync::Arc;
use std::time::Duration;

use super::tests::{marker, prepare};
use super::{DrainedTransition, RuntimeDrainController};
use crate::restart_readiness::drain_marker;

#[tokio::test(start_paused = true)]
async fn malformed_and_unreadable_restart_markers_never_release_the_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let controller = Arc::new(RuntimeDrainController::initialize(temp.path()).unwrap());
    let gate = controller.admission_gate();
    let key = prepare(&controller, "retry-restart-read");
    controller.claim_prepare().unwrap();
    controller.acknowledge(&key).unwrap();
    let restart = temp.path().join(".codex_discord_rust.restart");
    fs::write(&restart, "malformed").unwrap();

    let waiter = tokio::spawn({
        let controller = Arc::clone(&controller);
        let key = key.clone();
        async move { controller.wait_for_transition(&key).await }
    });
    tokio::time::advance(Duration::from_millis(101)).await;
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished(), "malformed restart must fail closed");
    assert!(gate.is_drained_for(&key));

    fs::remove_file(&restart).unwrap();
    fs::create_dir(&restart).unwrap();
    tokio::time::advance(Duration::from_millis(101)).await;
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished(), "restart read I/O error must retry");
    assert!(gate.is_drained_for(&key));

    fs::remove_dir(&restart).unwrap();
    fs::write(&restart, marker(&key, None)).unwrap();
    tokio::time::advance(Duration::from_millis(101)).await;
    assert_eq!(waiter.await.unwrap(), DrainedTransition::Restart);
}

#[tokio::test(start_paused = true)]
async fn failed_acknowledgement_cannot_release_before_a_later_exact_ack() {
    let temp = tempfile::tempdir().unwrap();
    let controller = Arc::new(RuntimeDrainController::initialize(temp.path()).unwrap());
    let gate = controller.admission_gate();
    let key = prepare(&controller, "retry-ack");
    controller.claim_prepare().unwrap();
    let ack = drain_marker::ack_path(temp.path());
    fs::create_dir(&ack).unwrap();
    fs::write(
        temp.path().join(".codex_discord_rust.restart"),
        marker(&key, None),
    )
    .unwrap();

    let waiter = tokio::spawn({
        let controller = Arc::clone(&controller);
        let key = key.clone();
        async move { controller.complete_drained_handshake(&key).await }
    });
    tokio::time::advance(Duration::from_millis(101)).await;
    tokio::task::yield_now().await;
    assert!(
        !waiter.is_finished(),
        "restart cannot precede the exact ACK"
    );
    assert!(gate.is_drained_for(&key));

    fs::remove_dir(&ack).unwrap();
    tokio::time::advance(Duration::from_millis(101)).await;
    assert_eq!(waiter.await.unwrap(), DrainedTransition::Restart);
    assert_eq!(drain_marker::read_ack(temp.path()).unwrap(), Some(key));
}

#[tokio::test(start_paused = true)]
async fn explicit_stop_remains_responsive_while_acknowledgement_fails() {
    let temp = tempfile::tempdir().unwrap();
    let controller = Arc::new(RuntimeDrainController::initialize(temp.path()).unwrap());
    let gate = controller.admission_gate();
    let key = prepare(&controller, "ack-failure-stop");
    controller.claim_prepare().unwrap();
    fs::create_dir(drain_marker::ack_path(temp.path())).unwrap();

    let waiter = tokio::spawn({
        let controller = Arc::clone(&controller);
        let key = key.clone();
        async move { controller.complete_drained_handshake(&key).await }
    });
    tokio::time::advance(Duration::from_millis(101)).await;
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());

    fs::write(temp.path().join(".codex_discord_rust.stop"), "stop").unwrap();
    tokio::time::advance(Duration::from_millis(101)).await;
    assert_eq!(waiter.await.unwrap(), DrainedTransition::Stop);
    assert!(
        gate.is_drained_for(&key),
        "stop never reopens the sealed gate"
    );
}
