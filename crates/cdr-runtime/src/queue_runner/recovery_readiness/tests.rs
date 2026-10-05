use super::*;
use crate::{
    app_backend::AppServerTurnBackend,
    restart_readiness::drain::{AdmissionGate, DrainFenceKey},
};
use cdr_app_server::ResidentAppServer;
use cdr_store::async_resolution::abandonment::Decision;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::async_orphan_tests::submitted_fixture::original as orphan;

#[allow(dead_code)]
#[path = "../../../../cdr-store/tests/support/recovery_readiness_fixture.rs"]
mod store_fixture;
use store_fixture::{Fixture as StoreFixture, ID, TARGET};

struct Fixture {
    temp: tempfile::TempDir,
    store: StoreFixture,
    server: Arc<ResidentAppServer>,
    backend: Arc<AppServerTurnBackend>,
}

fn script() -> Value {
    json!({"pages":[{"data":[{"id":"original","status":"completed","items":[]}],"nextCursor":null}],
        "goal_result":{"goal":null}})
}

impl Fixture {
    async fn new(script: &Value) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("history.json");
        std::fs::write(&source, script.to_string()).unwrap();
        let mut config = crate::test_support::native_fixture::config("async-history");
        config.environment.insert(
            "CDR_ASYNC_HISTORY_LOG".into(),
            temp.path().join("rpc.jsonl").to_string_lossy().into(),
        );
        config.environment.insert(
            "CDR_ASYNC_HISTORY_SCRIPT".into(),
            source.to_string_lossy().into(),
        );
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        let store = StoreFixture::with_runtime(Decision::AbandonOnly, server.instance_id());
        store.settle("original");
        let backend = Arc::new(AppServerTurnBackend::new(Arc::clone(&server)));
        Self {
            temp,
            store,
            server,
            backend,
        }
    }

    fn queue(&self, gate: &AdmissionGate) -> QueueCoordinator<AppServerTurnBackend> {
        QueueCoordinator::new_with_admission_gate(
            self.store.path.clone(),
            Arc::clone(&self.backend),
            gate.clone(),
        )
    }

    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.temp.path().join("rpc.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn read_only(&self) {
        assert!(self.calls().iter().all(|v| matches!(
            v["method"].as_str(),
            Some("initialize" | "thread/read" | "thread/turns/list" | "thread/goal/get")
        )));
        self.store.held();
    }
}

fn key() -> DrainFenceKey {
    DrainFenceKey::new("readiness-test", "1|2", "maintenance").unwrap()
}

async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("native observation phase was not reached");
}

#[tokio::test]
async fn native_queue_path_holds_target_and_commits_only_the_explicit_callback_once() {
    let mut f = Fixture::new(&script()).await;
    f.store
        .db
        .execute_batch("CREATE TABLE test_publication(value INTEGER)")
        .unwrap();
    let gate = AdmissionGate::new();
    let queue = f.queue(&gate);
    let before = f.store.state();
    let ready = queue
        .acquire_recovery_readiness(TARGET, ID, 1)
        .await
        .unwrap();
    let report = serde_json::to_value(ready.report()).unwrap();
    assert_eq!(report["release_authorized"], false);
    assert_eq!(report["execution_authorized"], false);
    assert_eq!(ready.snapshot().thread_id(), TARGET);
    assert!(queue.target_lock(TARGET).unwrap().try_lock_owned().is_err());
    assert!(
        queue
            .target_lock("unrelated")
            .unwrap()
            .try_lock_owned()
            .is_ok()
    );
    let tx = f
        .store
        .db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    tx.execute("INSERT INTO test_publication VALUES (1)", [])
        .unwrap();
    // Acquire/write outside both resident/lifecycle locks. Only final COMMIT is guarded.
    ready
        .with_current_connection(|| tx.commit())
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .db
            .query_row("SELECT count(*) FROM test_publication", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(queue.target_lock(TARGET).unwrap().try_lock_owned().is_ok());
    assert_eq!(f.store.state(), before);
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/read")
            .count(),
        2
    );
    f.read_only();
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn missing_gate_and_sealed_maintenance_do_not_read_native_state() {
    let f = Fixture::new(&script()).await;
    let without = QueueCoordinator::new(f.store.path.clone(), Arc::clone(&f.backend));
    assert!(
        without
            .acquire_recovery_readiness(TARGET, ID, 1)
            .await
            .is_err()
    );
    let gate = AdmissionGate::new();
    gate.seal(&key()).unwrap();
    let queue = f.queue(&gate);
    assert!(
        queue
            .acquire_recovery_readiness(TARGET, ID, 1)
            .await
            .is_err()
    );
    gate.close_controls(&key()).unwrap();
    assert!(
        queue
            .acquire_recovery_readiness(TARGET, ID, 1)
            .await
            .is_err()
    );
    assert!(gate.is_drained_for(&key()));
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] != "initialize")
            .count(),
        0
    );
    f.read_only();
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn maintenance_seal_after_observation_prevents_publication() {
    let f = Fixture::new(&script()).await;
    let gate = AdmissionGate::new();
    let queue = f.queue(&gate);
    let ready = queue
        .acquire_recovery_readiness(TARGET, ID, 1)
        .await
        .unwrap();
    gate.seal(&key()).unwrap();
    let count = AtomicUsize::new(0);
    assert!(
        ready
            .with_current_connection(|| count.fetch_add(1, Ordering::SeqCst))
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert!(gate.is_drained_for(&key()));
    f.read_only();
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn commit_first_then_maintenance_seal_is_ordered_without_reentrant_locking() {
    let f = Fixture::new(&script()).await;
    let gate = AdmissionGate::new();
    let queue = f.queue(&gate);
    let ready = queue
        .acquire_recovery_readiness(TARGET, ID, 1)
        .await
        .unwrap();
    let order = Arc::new(AtomicUsize::new(0));
    let (entered, waiting) = std::sync::mpsc::sync_channel(1);
    let mut worker = None;
    ready
        .with_current_connection(|| {
            let other_gate = gate.clone();
            let worker_order = Arc::clone(&order);
            worker = Some(std::thread::spawn(move || {
                entered.send(()).unwrap();
                other_gate.seal(&key()).unwrap();
                assert_eq!(worker_order.fetch_add(1, Ordering::SeqCst), 1);
            }));
            waiting.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(order.fetch_add(1, Ordering::SeqCst), 0);
        })
        .unwrap();
    worker.unwrap().join().unwrap();
    assert_eq!(order.load(Ordering::SeqCst), 2);
    assert!(gate.is_drained_for(&key()));
    f.read_only();
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn native_close_after_readiness_does_not_run_publication() {
    let f = Fixture::new(&script()).await;
    let gate = AdmissionGate::new();
    let queue = f.queue(&gate);
    let ready = queue
        .acquire_recovery_readiness(TARGET, ID, 1)
        .await
        .unwrap();
    f.server.close().await.unwrap();
    let count = AtomicUsize::new(0);
    assert!(
        ready
            .with_current_connection(|| count.fetch_add(1, Ordering::SeqCst))
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    f.read_only();
}

#[tokio::test]
async fn wrong_target_goal_turn_or_truncated_history_never_produces_readiness() {
    for mode in [
        "thread",
        "goal-missing",
        "goal-active",
        "goal-other",
        "turn",
        "missing",
        "truncated",
        "duplicate",
        "oversize",
        "cursor-repeat",
    ] {
        let mut value = script();
        match mode {
            "thread" => value["wrong_thread"] = json!("other"),
            "goal-missing" => value["goal_result"] = json!({}),
            "goal-active" => {
                value["goal_result"] = json!({"goal":{"threadId":TARGET,"status":"active"}});
            }
            "goal-other" => {
                value["goal_result"] = json!({"goal":{"threadId":"other","status":"complete"}});
            }
            "turn" => value["pages"][0]["data"][0]["status"] = json!("inProgress"),
            "missing" => value["pages"][0]["data"][0]["id"] = json!("different-owner"),
            "oversize" => value["pages"][0]["data"][0]["items"] = json!(["x".repeat(1_048_576)]),
            "cursor-repeat" => {
                value["pages"] = json!([{ "data":[],"nextCursor":"page-1" },{ "data":[],"nextCursor":"page-1" }]);
            }
            "truncated" => value["pages"][0]["truncated"] = json!(true),
            "duplicate" => {
                let t = value["pages"][0]["data"][0].clone();
                value["pages"][0]["data"].as_array_mut().unwrap().push(t);
            }
            _ => unreachable!(),
        }
        let f = Fixture::new(&value).await;
        assert!(
            f.queue(&AdmissionGate::new())
                .acquire_recovery_readiness(TARGET, ID, 1)
                .await
                .is_err(),
            "{mode}"
        );
        f.read_only();
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn bounded_owner_search_does_not_scan_forever_or_invent_absence_proof() {
    let f = Fixture::new(&json!({"endless":true,"goal_result":{"goal":null}})).await;
    assert!(
        f.queue(&AdmissionGate::new())
            .acquire_recovery_readiness(TARGET, ID, 1)
            .await
            .is_err()
    );
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/turns/list")
            .count(),
        8
    );
    f.read_only();
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn mapping_changed_while_native_goal_read_waits_is_revalidated() {
    let mut value = script();
    value["terminal_gate"] = json!(true);
    let f = Fixture::new(&value).await;
    let queue = f.queue(&AdmissionGate::new());
    let task = tokio::spawn(async move {
        queue
            .acquire_recovery_readiness(TARGET, ID, 1)
            .await
            .map(drop)
    });
    wait_file(&f.temp.path().join("rpc.jsonl.terminal-entered")).await;
    f.store
        .db
        .execute(
            "UPDATE mirror_threads SET discord_thread_id=21 WHERE codex_thread_id=?",
            [TARGET],
        )
        .unwrap();
    std::fs::write(
        f.temp.path().join("rpc.jsonl.terminal-release"),
        b"finish exact read",
    )
    .unwrap();
    let error = task.await.unwrap().err().unwrap();
    assert!(error.to_string().contains("mapping"), "{error}");
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/read")
            .count(),
        2
    );
    f.read_only();
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn fence_appearing_while_waiting_for_target_lock_prevents_all_native_reads() {
    let f = Fixture::new(&script()).await;
    let queue = f.queue(&AdmissionGate::new());
    let guard = queue.target_lock(TARGET).unwrap().lock_owned().await;
    let waiting = queue.acquire_recovery_readiness(TARGET, ID, 1);
    tokio::pin!(waiting);
    tokio::select! {
        biased;
        _ = &mut waiting => panic!("readiness bypassed the target lock"),
        () = tokio::task::yield_now() => {},
    }
    f.store
        .db
        .execute(
            "UPDATE mirror_threads SET discord_thread_id=21 WHERE codex_thread_id=?",
            [TARGET],
        )
        .unwrap();
    drop(guard);
    assert!(waiting.await.is_err());
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] != "initialize")
            .count(),
        0
    );
    f.read_only();
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_releases_target_control_and_native_read_without_replay() {
    let mut value = script();
    value["terminal_gate"] = json!(true);
    let f = Fixture::new(&value).await;
    let gate = AdmissionGate::new();
    let queue = Arc::new(f.queue(&gate));
    let task = tokio::spawn({
        let queue = Arc::clone(&queue);
        async move {
            queue
                .acquire_recovery_readiness(TARGET, ID, 1)
                .await
                .map(drop)
        }
    });
    wait_file(&f.temp.path().join("rpc.jsonl.terminal-entered")).await;
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(queue.target_lock(TARGET).unwrap().try_lock_owned().is_ok());
    gate.seal(&key()).unwrap();
    assert!(gate.is_drained_for(&key()));
    std::fs::write(
        f.temp.path().join("rpc.jsonl.terminal-release"),
        b"late observation only",
    )
    .unwrap();
    let other = f
        .server
        .execute(
            cdr_app_server::requests::read_thread("unrelated", false),
            Some(1),
        )
        .await
        .unwrap();
    assert_eq!(other["thread"]["id"], "unrelated");
    assert!(!f.server.lifecycle_snapshot().await.quarantined);
    f.read_only();
    f.server.close().await.unwrap();
}

struct LegacyBackend {
    reads: Arc<AtomicUsize>,
}
impl TurnBackend for LegacyBackend {
    fn generation(&self) -> u64 {
        1
    }
    fn resident_instance_id(&self) -> Option<&str> {
        Some(store_fixture::RUNTIME)
    }
    fn active_turn_id<'a>(
        &'a self,
        _target: &'a str,
    ) -> super::super::BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { panic!("legacy active owner query is not native readiness") })
    }
    fn read_turns<'a>(
        &'a self,
        _target: &'a str,
    ) -> super::super::BoxBackendFuture<'a, Vec<super::super::TurnRecord>> {
        Box::pin(async { panic!("legacy history is not native readiness") })
    }
    fn read_async_terminal<'a>(
        &'a self,
        _target: &'a str,
        _owners: &'a [String],
    ) -> super::super::BoxBackendFuture<'a, Option<Value>> {
        Box::pin(async {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(Some(store_fixture::observation("original")))
        })
    }
    fn resume_thread<'a>(&'a self, _target: &'a str) -> super::super::BoxBackendFuture<'a, ()> {
        Box::pin(async { panic!("readiness must not resume a thread") })
    }
    fn start_turn<'a>(
        &'a self,
        _target: &'a str,
        _prompt: &'a str,
    ) -> super::super::BoxBackendFuture<'a, String> {
        Box::pin(async { panic!("readiness must not start a turn") })
    }
}

#[tokio::test]
async fn caller_made_json_and_legacy_history_are_not_native_authority() {
    let f = StoreFixture::new(Decision::AbandonOnly);
    f.settle("original");
    let reads = Arc::new(AtomicUsize::new(0));
    let queue = QueueCoordinator::new_with_admission_gate(
        f.path.clone(),
        Arc::new(LegacyBackend {
            reads: Arc::clone(&reads),
        }),
        AdmissionGate::new(),
    );
    let error = queue
        .acquire_recovery_readiness(TARGET, ID, 1)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("unsupported"));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    f.held();
}
