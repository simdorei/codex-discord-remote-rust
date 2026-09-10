use std::fs;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cdr_app_server::{AppServerError, ResidentLifecycleSnapshot};
use cdr_store::mapping::upsert_thread;

use super::*;
use crate::restart_readiness::drain_controller::RuntimeDrainController;

struct FakeServer {
    active: Arc<AtomicBool>,
}

fn healthy_lifecycle() -> ResidentLifecycleSnapshot {
    ResidentLifecycleSnapshot {
        generation: 1,
        healthy: true,
        quarantined: false,
        restart_pending: false,
        process_id: Some(7),
    }
}

impl LiveDrainServer for FakeServer {
    async fn lifecycle(&self) -> ResidentLifecycleSnapshot {
        healthy_lifecycle()
    }

    async fn active_turn(&self, _thread_id: &str) -> Result<Option<String>, AppServerError> {
        Ok(self.active.load(Ordering::SeqCst).then(|| "turn-a".into()))
    }

    async fn has_unsettled_requests(&self) -> Result<bool, AppServerError> {
        Ok(false)
    }
}

struct ControlRaceServer {
    gate: crate::restart_readiness::drain::AdmissionGate,
    injected: AtomicBool,
    permit: Mutex<Option<crate::restart_readiness::drain::AdmissionPermit>>,
}

impl ControlRaceServer {
    fn release_control(&self) {
        self.permit.lock().unwrap().take();
    }
}

impl LiveDrainServer for ControlRaceServer {
    async fn lifecycle(&self) -> ResidentLifecycleSnapshot {
        if !self.injected.swap(true, Ordering::SeqCst) {
            let permit = self.gate.try_enter_control().unwrap();
            self.permit.lock().unwrap().replace(permit);
        }
        healthy_lifecycle()
    }

    async fn active_turn(&self, _thread_id: &str) -> Result<Option<String>, AppServerError> {
        Ok(None)
    }

    async fn has_unsettled_requests(&self) -> Result<bool, AppServerError> {
        Ok(false)
    }
}

#[tokio::test(start_paused = true)]
async fn live_turn_past_drain_deadline_never_becomes_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_owned();
    let db = root.join("mirror.sqlite");
    upsert_thread(&db, "thread-a", "project", "A", 10, 20, 1.0).unwrap();
    let gate = crate::restart_readiness::drain::AdmissionGate::new();
    let permit = gate.try_enter().unwrap();
    let key = DrainFenceKey::new("runtime-a", "42|99", "live-timeout").unwrap();
    gate.seal(&key).unwrap();
    let active = Arc::new(AtomicBool::new(true));
    let server = FakeServer {
        active: Arc::clone(&active),
    };

    let waiter = tokio::spawn(async move {
        drain_until_quiescent_inner(&root, &gate, &db, &server, key, Duration::from_millis(100))
            .await
    });
    tokio::time::advance(Duration::from_millis(101)).await;
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished(), "deadline must not request shutdown");

    drop(permit);
    active.store(false, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    let outcome = waiter.await.unwrap();
    assert!(matches!(outcome, GatewayLoopOutcome::Drained(_)));
}

#[tokio::test(start_paused = true)]
async fn final_control_timeout_reopens_controls_without_unsealing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_owned();
    let db = root.join("mirror.sqlite");
    upsert_thread(&db, "thread-a", "project", "A", 10, 20, 1.0).unwrap();
    let gate = crate::restart_readiness::drain::AdmissionGate::new();
    let key = DrainFenceKey::new("runtime-a", "42|99", "control-timeout").unwrap();
    gate.seal(&key).unwrap();
    let server = Arc::new(ControlRaceServer {
        gate: gate.clone(),
        injected: AtomicBool::new(false),
        permit: Mutex::new(None),
    });
    let task_gate = gate.clone();
    let task_server = Arc::clone(&server);

    let waiter = tokio::spawn(async move {
        drain_until_quiescent_inner(
            &root,
            &task_gate,
            &db,
            task_server.as_ref(),
            key,
            Duration::from_millis(100),
        )
        .await
    });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(101)).await;
    tokio::task::yield_now().await;

    let retry_control = gate
        .try_enter_control()
        .expect("same sealed fence must reopen controls after final timeout");
    assert!(gate.try_enter().is_err(), "normal ingress must stay sealed");
    assert!(!waiter.is_finished());

    server.release_control();
    drop(retry_control);
    for _ in 0..8 {
        tokio::time::advance(Duration::from_millis(250)).await;
        tokio::task::yield_now().await;
        if waiter.is_finished() {
            break;
        }
    }
    let outcome = waiter.await.unwrap();
    assert!(matches!(outcome, GatewayLoopOutcome::Drained(_)));
}

#[tokio::test(start_paused = true)]
async fn malformed_prepare_seals_ingress_without_stopping_the_gateway() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_owned();
    let controller = Arc::new(RuntimeDrainController::initialize(&root).unwrap());
    fs::write(
        crate::restart_readiness::drain_marker::prepare_path(&root),
        "not-a-drain-marker",
    )
    .unwrap();
    let gate = controller.admission_gate();
    let waiter = tokio::spawn(super::super::run_gateway_loop(
        super::super::GatewayLoopContext {
            operation_root: root.clone(),
            drain: controller,
            server: Arc::new(FakeServer {
                active: Arc::new(AtomicBool::new(false)),
            }),
            mirror_db: root.join("mirror.sqlite"),
        },
    ));
    tokio::task::yield_now().await;

    assert!(
        gate.try_enter().is_err(),
        "untrusted prepare must fail closed"
    );
    let control = gate
        .try_enter_control()
        .expect("existing work controls remain available while quarantined");
    assert!(
        !waiter.is_finished(),
        "marker failure must not close the runtime"
    );
    drop(control);

    fs::write(root.join(".codex_discord_rust.stop"), "stop").unwrap();
    tokio::time::advance(Duration::from_millis(251)).await;
    let outcome = waiter.await.unwrap().unwrap();
    assert!(matches!(outcome, GatewayLoopOutcome::Shutdown));
}

#[tokio::test(start_paused = true)]
async fn mismatched_prepare_preserves_an_existing_exact_seal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_owned();
    let controller = Arc::new(RuntimeDrainController::initialize(&root).unwrap());
    let gate = controller.admission_gate();
    let existing = DrainFenceKey::new("runtime-existing", "42|99", "existing").unwrap();
    gate.seal(&existing).unwrap();
    fs::write(
        crate::restart_readiness::drain_marker::prepare_path(&root),
        "version=1\nruntime_id=wrong-runtime\nprocess_identity=42|99\nnonce=wrong\n",
    )
    .unwrap();
    let waiter = tokio::spawn(super::super::run_gateway_loop(
        super::super::GatewayLoopContext {
            operation_root: root.clone(),
            drain: controller,
            server: Arc::new(FakeServer {
                active: Arc::new(AtomicBool::new(false)),
            }),
            mirror_db: root.join("mirror.sqlite"),
        },
    ));
    tokio::task::yield_now().await;

    assert!(gate.is_drained_for(&existing));
    assert!(
        !waiter.is_finished(),
        "identity mismatch must not stop the runtime"
    );

    fs::write(root.join(".codex_discord_rust.stop"), "stop").unwrap();
    tokio::time::advance(Duration::from_millis(251)).await;
    let outcome = waiter.await.unwrap().unwrap();
    assert!(matches!(outcome, GatewayLoopOutcome::Shutdown));
}
