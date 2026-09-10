use std::time::Duration;

use cdr_runtime::restart_readiness::drain::{AdmissionGate, DrainFenceKey};

const QUICK: Duration = Duration::from_millis(100);

fn key(runtime: &str, nonce: &str) -> DrainFenceKey {
    DrainFenceKey::new(runtime, "4100|638900000000000000", nonce).unwrap()
}

#[tokio::test]
async fn prepare_seals_before_ack_and_waits_for_every_in_flight_admission() {
    let gate = AdmissionGate::new();
    let first = gate.try_enter().expect("open gate admits work");
    let fence = key("runtime-a", "nonce-a");

    gate.seal(&fence).expect("first prepare seals the gate");
    assert!(
        gate.try_enter().is_err(),
        "new ingress is rejected after seal"
    );
    assert!(!gate.is_drained_for(&fence));

    drop(first);
    gate.wait_drained(&fence, QUICK)
        .await
        .expect("the exact fence drains");
    assert!(gate.is_drained_for(&fence));
}

#[tokio::test]
async fn exact_runtime_and_nonce_are_required_for_release_or_ack() {
    let gate = AdmissionGate::new();
    let active = key("runtime-a", "nonce-a");
    gate.seal(&active).unwrap();
    gate.wait_drained(&active, QUICK).await.unwrap();

    assert!(!gate.is_drained_for(&key("runtime-b", "nonce-a")));
    assert!(!gate.is_drained_for(&key("runtime-a", "nonce-b")));
    assert!(!gate.release(&key("runtime-b", "nonce-a")));
    assert!(!gate.release(&key("runtime-a", "nonce-b")));
    assert!(gate.try_enter().is_err(), "a stale release cannot unseal");

    assert!(gate.release(&active));
    assert!(gate.try_enter().is_ok());
}

#[tokio::test]
async fn drain_controls_are_tracked_until_the_exact_fence_closes_them() {
    let gate = AdmissionGate::new();
    let active = key("runtime-a", "nonce-controls");
    let stale = key("runtime-a", "stale-controls");
    gate.seal(&active).unwrap();

    let approval = gate
        .try_enter_control()
        .expect("approval/input control remains available while draining");
    assert!(!gate.is_drained_for(&active));
    assert!(gate.close_controls(&stale).is_err());
    assert!(gate.try_enter_control().is_ok());

    gate.close_controls(&active).unwrap();
    assert!(gate.try_enter_control().is_err());
    assert!(gate.open_controls(&stale).is_err());
    drop(approval);
    gate.wait_drained(&active, QUICK).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn drain_timeout_remains_sealed_and_visible() {
    let gate = AdmissionGate::new();
    let _in_flight = gate.try_enter().unwrap();
    let fence = key("runtime-a", "nonce-timeout");
    gate.seal(&fence).unwrap();

    let waiter = tokio::spawn({
        let gate = gate.clone();
        let fence = fence.clone();
        async move { gate.wait_drained(&fence, QUICK).await }
    });
    tokio::time::advance(QUICK).await;
    tokio::task::yield_now().await;

    assert!(waiter.await.unwrap().is_err());
    assert!(gate.try_enter().is_err(), "timeout must fail closed");
}
