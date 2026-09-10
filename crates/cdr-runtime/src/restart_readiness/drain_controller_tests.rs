use std::fs;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;

use super::*;
use crate::restart_readiness::drain_marker;

pub(super) fn marker(key: &DrainFenceKey, state: Option<&str>) -> String {
    format!(
        "version=1\nruntime_id={}\nprocess_identity={}\nnonce={}\n{}",
        key.runtime_id(),
        key.process_identity(),
        key.nonce(),
        state.map_or_else(String::new, |value| format!("state={value}\n"))
    )
}

pub(super) fn prepare(controller: &RuntimeDrainController, nonce: &str) -> DrainFenceKey {
    let identity = drain_marker::read_identity(&controller.root)
        .unwrap()
        .unwrap();
    let key = DrainFenceKey::new(
        identity.runtime_id,
        format!("{}|638900000000000000", identity.process_id),
        nonce,
    )
    .unwrap();
    fs::write(
        drain_marker::prepare_path(&controller.root),
        marker(&key, None),
    )
    .unwrap();
    key
}

#[tokio::test]
async fn controller_ack_is_exact_and_only_after_in_flight_admission_drains() {
    let temp = tempfile::tempdir().unwrap();
    let controller = RuntimeDrainController::initialize(temp.path()).unwrap();
    let gate = controller.admission_gate();
    let in_flight = gate.try_enter().unwrap();
    let key = prepare(&controller, "nonce-a");

    assert_eq!(controller.claim_prepare().unwrap(), Some(key.clone()));
    assert!(matches!(
        controller.acknowledge(&key),
        Err(DrainProtocolError::PrepareChanged)
    ));
    assert!(drain_marker::read_ack(temp.path()).unwrap().is_none());

    drop(in_flight);
    gate.wait_drained(&key, Duration::from_millis(100))
        .await
        .unwrap();
    controller.acknowledge(&key).unwrap();
    assert_eq!(drain_marker::read_ack(temp.path()).unwrap(), Some(key));
}

#[test]
fn stale_or_mismatched_prepare_never_claims_a_new_runtime() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        drain_marker::prepare_path(temp.path()),
        "version=1\nruntime_id=old-runtime\nprocess_identity=9|10\nnonce=old-nonce\n",
    )
    .unwrap();
    assert!(matches!(
        RuntimeDrainController::initialize(temp.path()),
        Err(DrainProtocolError::OrphanedState)
    ));

    fs::remove_file(drain_marker::prepare_path(temp.path())).unwrap();
    let controller = RuntimeDrainController::initialize(temp.path()).unwrap();
    let wrong = DrainFenceKey::new(
        "different-runtime",
        format!("{}|10", std::process::id()),
        "nonce",
    )
    .unwrap();
    fs::write(
        drain_marker::prepare_path(temp.path()),
        marker(&wrong, None),
    )
    .unwrap();
    assert!(matches!(
        controller.claim_prepare(),
        Err(DrainProtocolError::IdentityMismatch)
    ));
}

#[tokio::test]
async fn only_exact_bound_restart_releases_the_drained_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let controller = Arc::new(RuntimeDrainController::initialize(temp.path()).unwrap());
    let key = prepare(&controller, "nonce-bound");
    controller.claim_prepare().unwrap();
    controller.acknowledge(&key).unwrap();

    let waiter = tokio::spawn({
        let controller = Arc::clone(&controller);
        let key = key.clone();
        async move { controller.wait_for_transition(&key).await }
    });
    let stale =
        DrainFenceKey::new(key.runtime_id(), key.process_identity(), "stale-nonce").unwrap();
    fs::write(
        temp.path().join(".codex_discord_rust.restart"),
        marker(&stale, None),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !waiter.is_finished(),
        "stale marker cannot release the runtime"
    );

    fs::write(
        temp.path().join(".codex_discord_rust.restart"),
        marker(&key, None),
    )
    .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap(),
        DrainedTransition::Restart
    );
}
