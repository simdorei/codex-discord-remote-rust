use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cdr_runtime::action_executor::ActionExecutor;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::prompt_intake::list_prompt_intakes;
use cdr_store::queue::list;
use rusqlite::Connection;
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

#[derive(Default)]
pub struct IntakeBackend {
    pub active: AsyncMutex<BTreeMap<String, String>>,
    pub active_checks: AtomicUsize,
    pub become_busy_on_check: Option<usize>,
    pub fork_targets: BTreeMap<String, String>,
    pub fork_failures: Mutex<VecDeque<BackendFailure>>,
    pub fork_gate: Option<Arc<ForkGate>>,
    pub forks: AsyncMutex<Vec<String>>,
    pub starts: AsyncMutex<Vec<(String, String)>>,
}

pub struct ForkGate {
    db: PathBuf,
    pub entered: Semaphore,
    pub release: Semaphore,
    pub observed_intakes: AtomicUsize,
    pub observed_queue_jobs: AtomicUsize,
}

impl ForkGate {
    #[allow(dead_code)]
    pub fn new(db: PathBuf) -> Self {
        Self {
            db,
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
            observed_intakes: AtomicUsize::new(0),
            observed_queue_jobs: AtomicUsize::new(0),
        }
    }
}

impl TurnBackend for IntakeBackend {
    fn generation(&self) -> u64 {
        7
    }

    fn requires_app_server_fork(&self) -> bool {
        true
    }

    fn active_turn_id<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move {
            let check = self.active_checks.fetch_add(1, Ordering::SeqCst) + 1;
            if self
                .become_busy_on_check
                .is_some_and(|threshold| check >= threshold)
            {
                return Ok(Some("external-turn".into()));
            }
            Ok(self.active.lock().await.get(thread_id).cloned())
        })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn fork_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.forks.lock().await.push(thread_id.into());
            if let Some(gate) = &self.fork_gate {
                gate.observed_intakes.store(
                    list_prompt_intakes(&gate.db).unwrap().len(),
                    Ordering::SeqCst,
                );
                gate.observed_queue_jobs
                    .store(list(&gate.db).unwrap().len(), Ordering::SeqCst);
                gate.entered.add_permits(1);
                gate.release.acquire().await.unwrap().forget();
            }
            if let Some(failure) = self.fork_failures.lock().unwrap().pop_front() {
                return Err(failure);
            }
            self.fork_targets
                .get(thread_id)
                .cloned()
                .ok_or_else(|| BackendFailure::definite(format!("unexpected fork: {thread_id}")))
        })
    }

    fn start_turn<'a>(
        &'a self,
        thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts
                .lock()
                .await
                .push((thread_id.into(), prompt.into()));
            let turn = format!("turn-{thread_id}");
            self.active
                .lock()
                .await
                .insert(thread_id.into(), turn.clone());
            Ok(turn)
        })
    }
}

#[allow(dead_code)]
pub struct RecordingPreprocessor {
    pub seen: Mutex<Vec<(String, String)>>,
}

impl RecordingPreprocessor {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl PromptPreprocessor for RecordingPreprocessor {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            self.seen
                .lock()
                .unwrap()
                .push((prompt.into(), thread_id.into()));
            Ok(format!("prepared:{thread_id}:{prompt}"))
        })
    }
}

pub fn executor(
    temp: &tempfile::TempDir,
    db: PathBuf,
    bridge: Arc<BridgeState>,
    backend: Arc<IntakeBackend>,
    preprocessor: Arc<dyn PromptPreprocessor>,
) -> (
    Arc<QueueCoordinator<IntakeBackend>>,
    Arc<ActionExecutor<IntakeBackend>>,
) {
    let state = temp.path().join("state.sqlite");
    ensure_state_db(&state);
    let queue = Arc::new(QueueCoordinator::new(db.clone(), backend));
    let executor = Arc::new(
        ActionExecutor::new(state, db, bridge, Arc::clone(&queue))
            .with_prompt_preprocessor(preprocessor),
    );
    (queue, executor)
}

fn ensure_state_db(path: &Path) {
    if !path.exists() {
        Connection::open(path)
            .unwrap()
            .execute_batch(include_str!("../fixtures/action_state.sql"))
            .unwrap();
    }
}
