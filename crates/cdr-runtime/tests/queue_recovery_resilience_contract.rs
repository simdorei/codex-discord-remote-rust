use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::queue::{NewQueueJob, QueueJobState, begin_attempt, enqueue, list, mark_running};
use rusqlite::Connection;
use tokio::sync::Mutex;

#[derive(Default)]
struct TargetBackend {
    active: Mutex<BTreeMap<String, String>>,
    resume_failures: Mutex<BTreeSet<String>>,
    read_failures: Mutex<BTreeSet<String>>,
    resumes: Mutex<Vec<String>>,
    reads: Mutex<Vec<String>>,
    starts: Mutex<Vec<(String, String)>>,
    turns: Mutex<BTreeMap<String, BTreeMap<String, TurnStatus>>>,
}

impl TurnBackend for TargetBackend {
    fn generation(&self) -> u64 {
        7
    }

    fn active_turn_id<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move { Ok(self.active.lock().await.get(thread_id).cloned()) })
    }

    fn read_turns<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async move {
            self.reads.lock().await.push(thread_id.into());
            if self.read_failures.lock().await.contains(thread_id) {
                return Err(BackendFailure::definite("thread/read unavailable"));
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
            self.resumes.lock().await.push(thread_id.into());
            if self.resume_failures.lock().await.contains(thread_id) {
                return Err(BackendFailure::definite("thread/resume unavailable"));
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
            self.starts
                .lock()
                .await
                .push((thread_id.into(), prompt.into()));
            let turn_id = format!("new-{thread_id}");
            self.active
                .lock()
                .await
                .insert(thread_id.into(), turn_id.clone());
            Ok(turn_id)
        })
    }
}

fn job<'a>(id: &'a str, target: &'a str, generation: i64, created: f64) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 10,
        owner_user_id: Some(20),
        discord_message_id: None,
        app_server_generation: generation,
        prompt: id,
        queued: true,
        ack_sent: true,
        created_at: created,
    }
}

#[tokio::test]
async fn read_failure_is_nonfatal_and_later_targets_are_still_reconciled() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("job-a", "thread-a", 1, 1.0)).unwrap();
    begin_attempt(&db, "job-a", &[], 1).unwrap();
    mark_running(&db, "job-a", "turn-a", 1).unwrap();
    enqueue(&db, job("job-b", "thread-b", 2, 2.0)).unwrap();
    begin_attempt(&db, "job-b", &[], 2).unwrap();
    expire_starting(&db, "job-b");
    let backend = Arc::new(TargetBackend::default());
    backend.read_failures.lock().await.insert("thread-a".into());
    backend.turns.lock().await.insert(
        "thread-b".into(),
        BTreeMap::from([("turn-b".into(), TurnStatus::InProgress)]),
    );
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = coordinator.recover().await.unwrap();

    assert_eq!(report.unresolved, 1);
    assert_eq!(report.recovered_running, 1);
    assert_eq!(
        report.unavailable_targets,
        BTreeSet::from(["thread-a".into()])
    );
    assert_eq!(*backend.reads.lock().await, vec!["thread-a", "thread-b"]);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(jobs[0].app_server_generation, 1);
    assert_eq!(jobs[1].turn_id.as_deref(), Some("turn-b"));
    assert_eq!(jobs[1].state, QueueJobState::Running);
    assert_eq!(jobs[1].app_server_generation, 2);
    assert_eq!(jobs[1].execution_generation, Some(2));
    assert_eq!(jobs[1].attempt_count, 1);
    assert_eq!(jobs[0].execution_generation, Some(1));
    assert!(backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn starting_target_stays_unknown_while_a_later_target_can_start() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("job-a", "thread-a", 1, 1.0)).unwrap();
    begin_attempt(&db, "job-a", &[], 1).unwrap();
    expire_starting(&db, "job-a");
    enqueue(&db, job("job-b", "thread-b", 2, 2.0)).unwrap();
    let backend = Arc::new(TargetBackend::default());
    backend
        .resume_failures
        .lock()
        .await
        .insert("thread-a".into());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = coordinator.recover().await.unwrap();

    assert_eq!(report.requeued, 0);
    assert_eq!(report.unresolved, 1);
    assert_eq!(report.started, 1);
    assert!(report.unavailable_targets.is_empty());
    let jobs = list(&db).unwrap();
    assert_eq!(jobs[0].state, QueueJobState::Starting);
    assert_eq!(jobs[0].app_server_generation, 1);
    assert_eq!(jobs[0].attempt_count, 1);
    assert!(jobs[0].last_error.contains("does not authorize retry"));
    assert_eq!(jobs[1].state, QueueJobState::Running);
    assert_eq!(jobs[1].app_server_generation, 7);
    assert_eq!(
        *backend.starts.lock().await,
        vec![("thread-b".into(), "job-b".into())]
    );
    assert_eq!(*backend.resumes.lock().await, vec!["thread-b"]);
    assert_eq!(*backend.reads.lock().await, vec!["thread-a", "thread-b"]);
}

fn expire_starting(db: &std::path::Path, job_id: &str) {
    Connection::open(db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET updated_at = 0 WHERE job_id = ?",
            [job_id],
        )
        .unwrap();
}
