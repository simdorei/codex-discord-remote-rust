use std::time::{Duration, UNIX_EPOCH};

use cdr_store::processed::is_processed;
use tokio::sync::oneshot;

use super::tests::{config, message};
use super::{
    ErrorReportTarget, PreparedMessage, dispatch_message_create,
    prepare_message_create_at_with_gate,
};
use crate::restart_readiness::drain::{AdmissionGate, DrainFenceKey};

#[tokio::test]
async fn mcb_04_restart_drain_waits_for_full_message_processing() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let input = message(399, "hold processing");
    let report_target = ErrorReportTarget::from_message(&input);
    let gate = AdmissionGate::new();
    let worker_gate = gate.clone();
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();

    let worker = tokio::spawn(async move {
        dispatch_message_create(
            report_target,
            || {
                prepare_message_create_at_with_gate(
                    input,
                    None,
                    &config(),
                    &database,
                    UNIX_EPOCH + Duration::from_secs(500),
                    &worker_gate,
                    false,
                )
            },
            |_| async move {
                let _ = started_tx.send(());
                let _ = release_rx.await;
                Ok(())
            },
            |_, _| async {},
        )
        .await
    });
    started_rx.await.expect("processing started");
    let fence = DrainFenceKey::new("runtime-a", "42|99", "message-pause").unwrap();
    gate.seal(&fence).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            gate.wait_drained(&fence, Duration::from_secs(1)),
        )
        .await
        .is_err(),
        "processing retains its admission permit"
    );
    let _ = release_tx.send(());
    worker.await.unwrap().unwrap();
    gate.wait_drained(&fence, Duration::from_millis(100))
        .await
        .unwrap();
}

#[test]
fn mcb_05_pending_reply_window_never_admits_a_different_command() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let gate = AdmissionGate::new();
    let fence = DrainFenceKey::new("runtime-a", "42|99", "pending-only").unwrap();
    gate.seal(&fence).unwrap();

    let prepared = prepare_message_create_at_with_gate(
        message(400, "!new"),
        None,
        &config(),
        &database,
        UNIX_EPOCH + Duration::from_mins(10),
        &gate,
        true,
    )
    .unwrap_or_else(|_| panic!("classification succeeds"));

    assert!(matches!(prepared, PreparedMessage::Unavailable));
    assert!(!is_processed(&database, 400).expect("read replay state"));
}

#[test]
fn mcb_06_admitted_drain_answer_is_frozen_as_pending_reply_only() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let gate = AdmissionGate::new();
    let fence = DrainFenceKey::new("runtime-a", "42|99", "pending-answer").unwrap();
    gate.seal(&fence).unwrap();

    let prepared = prepare_message_create_at_with_gate(
        message(401, "pending answer"),
        None,
        &config(),
        &database,
        UNIX_EPOCH + Duration::from_secs(700),
        &gate,
        true,
    )
    .unwrap_or_else(|_| panic!("classification succeeds"));
    let PreparedMessage::Admitted(admitted, _permit) = prepared else {
        panic!("pending answer should receive the bounded control permit");
    };

    assert!(admitted.is_pending_reply_only());
}

#[test]
fn mcb_07_stop_remains_available_and_tracked_during_drain() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let gate = AdmissionGate::new();
    let fence = DrainFenceKey::new("runtime-a", "42|99", "message-stop").unwrap();
    gate.seal(&fence).unwrap();

    let prepared = prepare_message_create_at_with_gate(
        message(402, "!stop"),
        None,
        &config(),
        &database,
        UNIX_EPOCH + Duration::from_secs(800),
        &gate,
        false,
    )
    .unwrap_or_else(|_| panic!("stop classification succeeds"));
    let PreparedMessage::Admitted(admitted, permit) = prepared else {
        panic!("stop should receive a bounded control permit");
    };

    assert!(!admitted.is_pending_reply_only());
    assert!(!gate.is_drained_for(&fence));
    drop(permit);
    assert!(gate.is_drained_for(&fence));
}
