use std::collections::BTreeMap;
use std::sync::Arc;

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{
    BoxBackendFuture, BusyStatus, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::queue::{
    NewQueueJob, QueueJobState, begin_attempt, enqueue, list, mark_app_server_managed_target,
    record_start_failure,
};
use rusqlite::Connection;
use tokio::sync::Mutex;

const TARGET: &str = "thread-a";

#[derive(Default)]
struct GenerationBackend {
    active: Mutex<Option<String>>,
    starts: Mutex<Vec<String>>,
    turns: Mutex<BTreeMap<String, TurnStatus>>,
}

impl TurnBackend for GenerationBackend {
    fn generation(&self) -> u64 {
        2
    }

    fn requires_app_server_fork(&self) -> bool {
        true
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(self.active.lock().await.clone()) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
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

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            let mut starts = self.starts.lock().await;
            starts.push(prompt.into());
            let turn_id = format!("turn-{}", starts.len());
            drop(starts);
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
async fn old_ambiguous_start_blocks_new_generation_until_recovery_starts_old_once() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, TARGET, 2).unwrap();
    enqueue(&db, job("old", "old prompt", 1, 1.0)).unwrap();
    begin_attempt(&db, "old", &[], 1).unwrap();
    record_start_failure(&db, "old", 1, "generation one response was lost", true).unwrap();
    let backend = Arc::new(GenerationBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let submitted = queue
        .submit(TARGET, 10, 20, Some(2), "new prompt")
        .await
        .unwrap();

    assert!(submitted.queued);
    assert_eq!(submitted.turn_id, None);
    assert!(backend.starts.lock().await.is_empty());
    let first_recovery = queue.recover().await.unwrap();
    assert_eq!(first_recovery.requeued, 1);
    assert!(backend.starts.lock().await.is_empty());
    expire_retry(&db, "old");

    let second_recovery = queue.recover().await.unwrap();

    assert_eq!(second_recovery.started, 1);
    assert_eq!(backend.starts.lock().await.as_slice(), &["old prompt"]);
    let jobs = list(&db).unwrap();
    let old = jobs.iter().find(|job| job.job_id == "old").unwrap();
    let new = jobs
        .iter()
        .find(|job| job.discord_message_id == Some(2))
        .unwrap();
    assert_eq!(old.state, QueueJobState::Running);
    assert_eq!(old.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(new.state, QueueJobState::Pending);
    assert_eq!(new.turn_id, None);
    assert_eq!(jobs.iter().filter(|job| job.turn_id.is_some()).count(), 1);
}

#[tokio::test]
async fn old_generation_pending_is_busy_and_cannot_be_overtaken_by_submit() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, TARGET, 2).unwrap();
    enqueue(&db, job("old", "old prompt", 1, 1.0)).unwrap();
    let backend = Arc::new(GenerationBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    assert_eq!(
        queue.busy_status(TARGET).await.unwrap(),
        BusyStatus {
            busy: true,
            allow_steer: false,
        }
    );
    let submitted = queue
        .submit(TARGET, 10, 20, Some(3), "new prompt")
        .await
        .unwrap();

    assert!(submitted.queued);
    assert_eq!(submitted.turn_id, None);
    assert!(backend.starts.lock().await.is_empty());
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|job| job.state == QueueJobState::Pending));
}

#[tokio::test]
async fn quarantined_old_generation_does_not_block_a_new_submission() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    mark_app_server_managed_target(&db, TARGET, 2).unwrap();
    enqueue(&db, job("isolated", "must not run", 1, 1.0)).unwrap();
    Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET state = 'running', \
             turn_id = 'cdr-quarantined:test', \
             last_error = '[cdr-rust:app-server-fork-quarantine:v1] isolated' \
             WHERE job_id = 'isolated'",
            [],
        )
        .unwrap();
    let backend = Arc::new(GenerationBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    assert_eq!(
        queue.busy_status(TARGET).await.unwrap(),
        BusyStatus {
            busy: false,
            allow_steer: false,
        }
    );
    let submitted = queue
        .submit(TARGET, 10, 20, Some(4), "safe new prompt")
        .await
        .unwrap();

    assert!(!submitted.queued);
    assert_eq!(submitted.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(backend.starts.lock().await.as_slice(), &["safe new prompt"]);
    let jobs = list(&db).unwrap();
    assert_eq!(
        jobs.iter()
            .filter(|job| job.state == QueueJobState::Quarantined)
            .count(),
        1
    );
    assert_eq!(
        jobs.iter()
            .filter(|job| job.state == QueueJobState::Running)
            .count(),
        1
    );
}

fn job(
    job_id: &'static str,
    prompt: &'static str,
    generation: i64,
    created_at: f64,
) -> NewQueueJob<'static> {
    NewQueueJob {
        job_id,
        target_thread_id: TARGET,
        channel_id: 10,
        owner_user_id: Some(20),
        discord_message_id: Some(1),
        app_server_generation: generation,
        prompt,
        queued: true,
        ack_sent: true,
        created_at,
    }
}

fn expire_retry(db: &std::path::Path, job_id: &str) {
    Connection::open(db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET updated_at = 0 WHERE job_id = ?",
            [job_id],
        )
        .unwrap();
}
