#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::queue::{NewQueueJob, enqueue};
use rusqlite::Connection;
use tokio::sync::Mutex;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Calls {
    pub active: Vec<String>,
    pub resumes: Vec<String>,
    pub reads: Vec<String>,
    pub starts: Vec<(String, String)>,
}

#[derive(Default)]
pub struct BackoffBackend {
    pub calls: Mutex<Calls>,
    pub active: Mutex<BTreeMap<String, String>>,
    pub resume_failures: Mutex<BTreeSet<String>>,
    pub read_failures: Mutex<BTreeSet<String>>,
    pub start_failures: Mutex<BTreeMap<String, VecDeque<BackendFailure>>>,
    pub turns: Mutex<BTreeMap<String, BTreeMap<String, TurnStatus>>>,
}

impl BackoffBackend {
    pub async fn snapshot(&self) -> Calls {
        self.calls.lock().await.clone()
    }
}

impl TurnBackend for BackoffBackend {
    fn generation(&self) -> u64 {
        7
    }

    fn active_turn_id<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move {
            self.calls.lock().await.active.push(thread_id.into());
            Ok(self.active.lock().await.get(thread_id).cloned())
        })
    }

    fn read_turns<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async move {
            self.calls.lock().await.reads.push(thread_id.into());
            if self.read_failures.lock().await.contains(thread_id) {
                return Err(BackendFailure::definite(format!(
                    "read failed for {thread_id}"
                )));
            }
            Ok(self
                .turns
                .lock()
                .await
                .get(thread_id)
                .into_iter()
                .flat_map(|turns| turns.iter())
                .map(|(turn_id, status)| TurnRecord {
                    turn_id: turn_id.clone(),
                    status: *status,
                })
                .collect())
        })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.calls.lock().await.resumes.push(thread_id.into());
            if self.resume_failures.lock().await.contains(thread_id) {
                return Err(BackendFailure::definite(format!(
                    "resume failed for {thread_id}"
                )));
            }
            Ok(())
        })
    }

    fn start_turn<'a>(
        &'a self,
        thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.calls
                .lock()
                .await
                .starts
                .push((thread_id.into(), prompt.into()));
            if let Some(failure) = self
                .start_failures
                .lock()
                .await
                .get_mut(thread_id)
                .and_then(VecDeque::pop_front)
            {
                return Err(failure);
            }
            let turn_id = format!("turn-{thread_id}");
            self.active
                .lock()
                .await
                .insert(thread_id.into(), turn_id.clone());
            self.turns
                .lock()
                .await
                .entry(thread_id.into())
                .or_default()
                .insert(turn_id.clone(), TurnStatus::InProgress);
            Ok(turn_id)
        })
    }
}

pub fn coordinator(
    db: &std::path::Path,
    backend: &Arc<BackoffBackend>,
) -> QueueCoordinator<BackoffBackend> {
    QueueCoordinator::new(db.to_path_buf(), Arc::clone(backend))
}

pub fn enqueue_job(
    db: &std::path::Path,
    job_id: &str,
    target: &str,
    message_id: i64,
    created_at: f64,
) {
    enqueue(
        db,
        NewQueueJob {
            job_id,
            target_thread_id: target,
            channel_id: 10,
            owner_user_id: Some(20),
            discord_message_id: Some(message_id),
            app_server_generation: 7,
            prompt: job_id,
            queued: true,
            ack_sent: true,
            created_at,
        },
    )
    .unwrap();
}

pub fn make_due(db: &std::path::Path, job_id: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    Connection::open(db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET updated_at = ? WHERE job_id = ?",
            rusqlite::params![now - 901.0, job_id],
        )
        .unwrap();
}
