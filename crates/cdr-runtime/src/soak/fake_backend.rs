use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use cdr_app_server::outcomes::TurnStatus;
use tokio::sync::Mutex;

use crate::queue_runner::{BackendFailure, BoxBackendFuture, TurnBackend, TurnRecord};

pub(super) struct FakeTurnBackend {
    generation: AtomicU64,
    restart_epoch: AtomicU64,
    resume_unavailable: StdMutex<BTreeSet<String>>,
    state: Mutex<BackendState>,
}

#[derive(Default)]
struct BackendState {
    next_turn: u64,
    targets: BTreeMap<String, TargetState>,
    resume_attempts: BTreeMap<String, u64>,
}

#[derive(Default)]
struct TargetState {
    active: Option<String>,
    starts: u64,
    turns: BTreeMap<String, TurnStatus>,
}

impl FakeTurnBackend {
    pub fn set_resume_unavailable(&self, target: &str, unavailable: bool) {
        let mut targets = self
            .resume_unavailable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if unavailable {
            targets.insert(target.into());
        } else {
            targets.remove(target);
        }
    }

    pub fn restart_reusing_generation(&self) -> (u64, u64) {
        let generation = self.generation.load(Ordering::SeqCst);
        self.generation.store(generation, Ordering::SeqCst);
        let restart_epoch = self.restart_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        (generation, restart_epoch)
    }

    pub async fn is_active(&self, target: &str) -> bool {
        self.state
            .lock()
            .await
            .targets
            .get(target)
            .is_some_and(|state| state.active.is_some())
    }

    pub async fn finish_active(&self, target: &str) -> Result<String, BackendFailure> {
        let mut state = self.state.lock().await;
        let target_state = state
            .targets
            .get_mut(target)
            .ok_or_else(|| BackendFailure::definite(format!("missing target {target}")))?;
        let turn_id = target_state
            .active
            .take()
            .ok_or_else(|| BackendFailure::definite(format!("target {target} is not active")))?;
        target_state.turns.remove(&turn_id);
        Ok(turn_id)
    }

    pub async fn starts(&self, target: &str) -> u64 {
        self.state
            .lock()
            .await
            .targets
            .get(target)
            .map_or(0, |state| state.starts)
    }

    pub async fn resume_attempts(&self, target: &str) -> u64 {
        self.state
            .lock()
            .await
            .resume_attempts
            .get(target)
            .copied()
            .unwrap_or_default()
    }
}

impl Default for FakeTurnBackend {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(1),
            restart_epoch: AtomicU64::new(0),
            resume_unavailable: StdMutex::new(BTreeSet::new()),
            state: Mutex::new(BackendState::default()),
        }
    }
}

impl TurnBackend for FakeTurnBackend {
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn active_turn_id<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move {
            Ok(self
                .state
                .lock()
                .await
                .targets
                .get(thread_id)
                .and_then(|target| target.active.clone()))
        })
    }

    fn read_turns<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async move {
            Ok(self
                .state
                .lock()
                .await
                .targets
                .get(thread_id)
                .map(|target| {
                    target
                        .turns
                        .iter()
                        .map(|(turn_id, status)| TurnRecord {
                            turn_id: turn_id.clone(),
                            status: *status,
                        })
                        .collect()
                })
                .unwrap_or_default())
        })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            *self
                .state
                .lock()
                .await
                .resume_attempts
                .entry(thread_id.into())
                .or_default() += 1;
            if self
                .resume_unavailable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(thread_id)
            {
                return Err(BackendFailure::definite(format!(
                    "deterministic resume outage for {thread_id}"
                )));
            }
            self.state
                .lock()
                .await
                .targets
                .entry(thread_id.into())
                .or_default();
            Ok(())
        })
    }

    fn start_turn<'a>(
        &'a self,
        thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            state.next_turn += 1;
            let turn_id = format!("fake-turn-{}", state.next_turn);
            let target = state.targets.entry(thread_id.into()).or_default();
            if target.active.is_some() {
                return Err(BackendFailure::definite(format!(
                    "target {thread_id} already has an active turn"
                )));
            }
            target.active = Some(turn_id.clone());
            target.starts += 1;
            target.turns.insert(turn_id.clone(), TurnStatus::InProgress);
            Ok(turn_id)
        })
    }
}
