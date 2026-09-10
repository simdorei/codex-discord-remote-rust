use std::collections::BTreeMap;
use std::sync::Arc;

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use cdr_runtime::restart_readiness::drain::{AdmissionGate, DrainFenceKey};
use cdr_store::queue::{QueueJobState, list};
use tokio::sync::Mutex;

#[derive(Default)]
struct FakeBackend {
    active: Mutex<Option<String>>,
    starts: Mutex<Vec<String>>,
    turns: Mutex<BTreeMap<String, TurnStatus>>,
}

impl TurnBackend for FakeBackend {
    fn generation(&self) -> u64 {
        7
    }

    fn active_turn_id<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(self.active.lock().await.clone()) })
    }

    fn read_turns<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async {
            Ok(self
                .turns
                .lock()
                .await
                .iter()
                .map(|(turn_id, status)| TurnRecord {
                    turn_id: turn_id.clone(),
                    status: *status,
                })
                .collect())
        })
    }

    fn resume_thread<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn start_turn<'a>(&'a self, _thread: &'a str, prompt: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts.lock().await.push(prompt.into());
            let turn_id = format!("turn-{}", self.starts.lock().await.len());
            *self.active.lock().await = Some(turn_id.clone());
            self.turns
                .lock()
                .await
                .insert(turn_id.clone(), TurnStatus::InProgress);
            Ok(turn_id)
        })
    }
}

#[tokio::test]
async fn completion_under_seal_keeps_the_next_prompt_pending() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("mirror.sqlite");
    let backend = Arc::new(FakeBackend::default());
    let admission = AdmissionGate::new();
    let coordinator = QueueCoordinator::new_with_admission_gate(
        db.clone(),
        Arc::clone(&backend),
        admission.clone(),
    );
    coordinator
        .submit("thread-a", 10, 20, Some(102), "first")
        .await
        .unwrap();
    coordinator
        .submit("thread-a", 10, 20, Some(103), "second")
        .await
        .unwrap();
    admission
        .seal(&DrainFenceKey::new("runtime-a", "42|99", "completion").unwrap())
        .unwrap();

    *backend.active.lock().await = None;
    coordinator
        .stage_turn_completion("thread-a", "turn-1", "finished")
        .await
        .unwrap();

    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].prompt, "second");
    assert_eq!(jobs[0].state, QueueJobState::Pending);
    assert_eq!(*backend.starts.lock().await, vec!["first"]);
}
