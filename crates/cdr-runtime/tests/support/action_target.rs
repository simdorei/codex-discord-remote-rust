use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cdr_runtime::action_executor::ActionExecutor;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, begin_app_server_fork_handoff,
    complete_app_server_fork_handoff,
};
use rusqlite::Connection;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct FakeBackend {
    pub active: Mutex<BTreeMap<String, String>>,
    pub forks: Mutex<Vec<String>>,
    pub fork_targets: BTreeMap<String, String>,
    pub resume_conflicts: BTreeSet<String>,
    pub resume_hangs: BTreeSet<String>,
    pub resumes: Mutex<Vec<String>>,
    pub starts: Mutex<Vec<(String, String)>>,
}

impl TurnBackend for FakeBackend {
    fn generation(&self) -> u64 {
        7
    }

    fn requires_app_server_fork(&self) -> bool {
        true
    }

    fn active_turn_id<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move { Ok(self.active.lock().await.get(thread_id).cloned()) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.resumes.lock().await.push(thread_id.into());
            if self.resume_hangs.contains(thread_id) {
                std::future::pending::<()>().await;
            }
            if self.resume_conflicts.contains(thread_id) {
                return Err(BackendFailure::active_writer(format!(
                    "thread/resume failed: thread {thread_id} already has an active writer"
                )));
            }
            Ok(())
        })
    }

    fn fork_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.forks.lock().await.push(thread_id.into());
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

pub fn executor(
    temp: &tempfile::TempDir,
    db: PathBuf,
    bridge: Arc<BridgeState>,
    backend: Arc<FakeBackend>,
) -> ActionExecutor<FakeBackend> {
    let state = temp.path().join("state.sqlite");
    if !state.exists() {
        Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("../fixtures/action_state.sql"))
            .unwrap();
    }
    let queue = Arc::new(QueueCoordinator::new(db.clone(), backend));
    ActionExecutor::new(state, db, bridge, queue)
}

#[allow(dead_code)]
pub fn seed_handoff(
    db: &Path,
    handoff: &str,
    source: &str,
    target: &str,
    channel: i64,
    thread: i64,
) {
    upsert_thread(db, source, "project", "Title", channel, thread, 1.0).unwrap();
    begin_app_server_fork_handoff(
        db,
        NewAppServerForkHandoff {
            handoff_id: handoff,
            ambiguous_job_id: None,
            source_thread_id: source,
            expected_generation: 7,
            quarantine_reason: "seed",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(db, handoff, target, 7).unwrap();
}

#[allow(dead_code)]
pub fn job<'a>(
    id: &'a str,
    target: &'a str,
    channel: i64,
    prompt: &'a str,
    created_at: f64,
) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: channel,
        owner_user_id: Some(20),
        discord_message_id: None,
        app_server_generation: 7,
        prompt,
        queued: true,
        ack_sent: true,
        created_at,
    }
}
