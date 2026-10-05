use super::*;
use crate::NativeRecoveryObservation;
use std::sync::atomic::{AtomicUsize, Ordering};

fn observe(
    f: &Fixture,
    wait: Duration,
) -> JoinHandle<Result<NativeRecoveryObservation, AppServerError>> {
    let server = Arc::clone(&f.server);
    tokio::spawn(async move {
        server
            .observe_recovery_prerequisites("thread-a", &["original".into()], wait)
            .await
    })
}

async fn respond(f: &mut Fixture, request: &Value, cursor: Value) {
    let result = match request["method"].as_str().unwrap() {
        "thread/read" => json!({"thread":{"id":"thread-a","status":{"type":"idle"}}}),
        "thread/turns/list" => {
            json!({"data":[{"id":"original","status":"completed","items":[]}],"nextCursor":cursor})
        }
        "thread/goal/get" => json!({"goal":null}),
        other => panic!("unexpected recovery RPC: {other}"),
    };
    f.send(json!({"id":request["id"],"result":result})).await;
}

async fn completed(f: &mut Fixture, cursor: Value) -> NativeRecoveryObservation {
    let task = observe(f, BARRIER);
    for expected in [
        "thread/read",
        "thread/turns/list",
        "thread/goal/get",
        "thread/read",
    ] {
        let request = f.next().await;
        assert_eq!(request["method"], expected);
        assert_eq!(request["params"]["threadId"], "thread-a");
        respond(f, &request, cursor.clone()).await;
    }
    task.await.unwrap().unwrap()
}

#[tokio::test]
async fn native_pin_is_one_use_and_does_not_claim_history_exhaustion() {
    let mut f = Fixture::new();
    let proof = completed(&mut f, json!("more-history")).await;
    assert_eq!(proof.resident_id(), f.server.instance_id());
    assert_eq!(proof.thread_id(), "thread-a");
    assert_eq!(proof.generation(), 1);
    assert_eq!(proof.observation()["required_owners_complete"], true);
    assert_eq!(proof.observation()["truncated"], false);
    assert_eq!(proof.observation()["history_exhausted"], false);
    let published = AtomicUsize::new(0);
    assert_eq!(
        proof
            .with_current_connection(|| {
                published.fetch_add(1, Ordering::SeqCst);
                7
            })
            .unwrap(),
        7
    );
    assert_eq!(published.load(Ordering::SeqCst), 1);
    assert_eq!(f.pending(), 0);
    f.healthy_b().await;
}

#[tokio::test]
async fn invalidation_before_publication_never_runs_the_callback() {
    for mode in [
        "quarantine",
        "restart",
        "close",
        "same-generation-client",
        "replacement",
    ] {
        let mut f = Fixture::new();
        let foreign = Fixture::new();
        let proof = completed(&mut f, Value::Null).await;
        match mode {
            "quarantine" => f.server.state.mark_cancelled(1),
            "restart" => f.server.state.request_restart(),
            "close" => f.client.seal_admissions(),
            "same-generation-client" => {
                f.server.state.inner.lock().unwrap().client = Some(foreign.client.clone());
            }
            "replacement" => {
                f.server
                    .state
                    .record_replacement(&foreign.client, 2)
                    .unwrap();
                f.server
                    .state
                    .install_replacement(&foreign.client, 2)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let count = AtomicUsize::new(0);
        assert!(
            proof
                .with_current_connection(|| count.fetch_add(1, Ordering::SeqCst))
                .is_err(),
            "{mode}"
        );
        assert_eq!(count.load(Ordering::SeqCst), 0, "{mode}");
    }
}

#[tokio::test]
async fn publication_and_resident_or_transport_invalidation_are_serialized() {
    for transport in [false, true] {
        let mut f = Fixture::new();
        let proof = completed(&mut f, Value::Null).await;
        let state = f.server.state.clone();
        let client = f.client.clone();
        let order = Arc::new(AtomicUsize::new(0));
        let (entered, waiting) = std::sync::mpsc::sync_channel(1);
        let mut worker = None;
        proof
            .with_current_connection(|| {
                assert!(matches!(
                    f.server.state.inner.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                let worker_order = Arc::clone(&order);
                worker = Some(std::thread::spawn(move || {
                    entered.send(()).unwrap();
                    if transport {
                        crate::transport::mark_closed(&client.inner, "controlled close");
                    } else {
                        state.mark_cancelled(1);
                    }
                    assert_eq!(
                        worker_order.fetch_add(1, Ordering::SeqCst),
                        1,
                        "invalidation ran before commit"
                    );
                }));
                waiting.recv_timeout(BARRIER).unwrap();
                assert_eq!(
                    order.fetch_add(1, Ordering::SeqCst),
                    0,
                    "commit was not first"
                );
            })
            .unwrap();
        worker.unwrap().join().unwrap();
        assert_eq!(order.load(Ordering::SeqCst), 2);
        assert!(
            f.server
                .state
                .with_recovery_current(&f.client, 1, || ())
                .is_err()
        );
    }
}

#[tokio::test]
async fn quarantine_during_a_native_read_cannot_produce_a_pinned_result() {
    let mut f = Fixture::new();
    let task = observe(&f, BARRIER);
    let first = f.next().await;
    respond(&mut f, &first, Value::Null).await;
    let history = f.next().await;
    assert_eq!(history["method"], "thread/turns/list");
    f.server.state.mark_cancelled(1);
    respond(&mut f, &history, Value::Null).await;
    assert!(task.await.unwrap().is_err());
    assert_eq!(f.pending(), 0);
    assert!(
        timeout(Duration::from_millis(20), f.wire.next_line())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pinned_read_timeout_is_local_and_other_target_remains_healthy() {
    let mut f = Fixture::new();
    let task = observe(&f, EXPIRES);
    assert_eq!(f.next().await["method"], "thread/read");
    assert!(matches!(
        task.await.unwrap(),
        Err(AppServerError::Timeout { .. })
    ));
    assert_eq!(f.pending(), 0);
    assert!(!f.server.lifecycle_snapshot().await.quarantined);
    f.healthy_b().await;
}

#[tokio::test]
async fn cancelling_a_flushed_pinned_read_reclaims_pending_and_allows_other_work() {
    let mut f = Fixture::new();
    let task = observe(&f, BARRIER);
    let first = f.next().await;
    drop(f.client.inner.stdin.lock().await);
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(f.pending(), 0);
    respond(&mut f, &first, Value::Null).await;
    f.barrier().await;
    f.healthy_b().await;
}

#[tokio::test]
async fn pinned_partial_write_cancellation_keeps_the_original_quarantine_rule() {
    let pause = Arc::new(WriteTestPause::new());
    let server = Arc::new(test_server(Arc::clone(&pause)));
    let task = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .observe_recovery_prerequisites("thread-a", &["original".into()], BARRIER)
                .await
        }
    });
    timeout(BARRIER, pause.wait_until_entered()).await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    let state = server.lifecycle_snapshot().await;
    timeout(Duration::from_secs(3), server.close())
        .await
        .unwrap()
        .unwrap();
    assert!(state.quarantined && state.restart_pending);
}

#[tokio::test]
async fn callback_panic_releases_both_native_locks_and_the_admission() {
    let mut f = Fixture::new();
    let proof = completed(&mut f, Value::Null).await;
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), AppServerError> =
            proof.with_current_connection(|| panic!("injected commit callback panic"));
    }));
    assert!(failed.is_err());
    drop(f.server.state.inner.lock().unwrap());
    assert_eq!(f.pending(), 0);
    f.healthy_b().await;
}

#[tokio::test]
async fn expired_observation_cannot_publish_or_recreate_authority() {
    let mut f = Fixture::new();
    let proof = completed(&mut f, Value::Null).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    let count = AtomicUsize::new(0);
    assert!(
        proof
            .with_current_connection(|| count.fetch_add(1, Ordering::SeqCst))
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    f.healthy_b().await;
}
