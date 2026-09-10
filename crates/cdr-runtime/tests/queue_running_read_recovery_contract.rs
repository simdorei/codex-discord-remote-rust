use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::delivery::list_pending;
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{NewQueueJob, QueueJobState, begin_attempt, enqueue, list, mark_running};
use tokio::sync::Mutex;

#[derive(Default)]
struct ReadFirstBackend {
    app_server_only: bool,
    active_writer: Mutex<BTreeSet<String>>,
    read_failures: Mutex<BTreeSet<String>>,
    turns: Mutex<BTreeMap<String, BTreeMap<String, TurnStatus>>>,
    reads: Mutex<Vec<String>>,
    resumes: Mutex<Vec<String>>,
    forks: Mutex<Vec<String>>,
    starts: Mutex<Vec<String>>,
}

impl TurnBackend for ReadFirstBackend {
    fn generation(&self) -> u64 {
        9
    }

    fn requires_app_server_fork(&self) -> bool {
        self.app_server_only
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
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
            if self.active_writer.lock().await.contains(thread_id) {
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
            Ok(format!("fork-{thread_id}"))
        })
    }

    fn start_turn<'a>(
        &'a self,
        thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts.lock().await.push(thread_id.into());
            Ok(format!("new-{thread_id}"))
        })
    }
}

#[tokio::test]
async fn completed_running_is_read_and_staged_before_pending_work_is_forked() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "desktop-owned", "project", "Original", 70, 71, 1.0).unwrap();
    enqueue(&db, job("running", "desktop-owned", 1, 1.0)).unwrap();
    begin_attempt(&db, "running", &[], 1).unwrap();
    mark_running(&db, "running", "turn-done", 1).unwrap();
    enqueue(&db, job("pending", "desktop-owned", 1, 2.0)).unwrap();
    let backend = Arc::new(ReadFirstBackend {
        app_server_only: true,
        ..ReadFirstBackend::default()
    });
    backend
        .active_writer
        .lock()
        .await
        .insert("desktop-owned".into());
    backend.turns.lock().await.insert(
        "desktop-owned".into(),
        BTreeMap::from([("turn-done".into(), TurnStatus::Completed)]),
    );
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = queue.recover().await.unwrap();

    assert_eq!(*backend.reads.lock().await, vec!["desktop-owned"]);
    assert!(backend.resumes.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
    assert!(report.read_unavailable_targets.is_empty());
    assert_eq!(report.unresolved, 1);

    let stage_error = queue
        .stage_turn_completion("desktop-owned", "turn-done", "final answer")
        .await
        .unwrap_err();
    assert!(stage_error.to_string().contains("active writer"));
    assert_eq!(list_pending(&db).unwrap().len(), 1);
    assert_eq!(list(&db).unwrap()[0].state, QueueJobState::Pending);

    let resumed = queue.recover().await.unwrap();

    assert_eq!(resumed.started, 1);
    assert_eq!(*backend.forks.lock().await, vec!["desktop-owned"]);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "fork-desktop-owned");
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(list_pending(&db).unwrap().len(), 1);
}

#[tokio::test]
async fn running_read_failure_is_distinct_and_never_attempts_resume() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("running", "thread-a", 1, 1.0)).unwrap();
    begin_attempt(&db, "running", &[], 1).unwrap();
    mark_running(&db, "running", "turn-a", 1).unwrap();
    let backend = Arc::new(ReadFirstBackend::default());
    backend.read_failures.lock().await.insert("thread-a".into());
    let queue = QueueCoordinator::new(db, Arc::clone(&backend));

    let report = queue.recover().await.unwrap();

    assert_eq!(
        report.read_unavailable_targets,
        BTreeSet::from(["thread-a".into()])
    );
    assert!(report.mutation_unavailable_targets.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
}

#[tokio::test]
async fn mutation_retry_backoff_never_blocks_a_later_running_read() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("work", "thread-a", 9, 1.0)).unwrap();
    let backend = Arc::new(ReadFirstBackend::default());
    backend.active_writer.lock().await.insert("thread-a".into());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let unavailable = queue.recover().await.unwrap();
    assert!(
        unavailable
            .mutation_unavailable_targets
            .contains("thread-a")
    );
    begin_attempt(&db, "work", &[], 9).unwrap();
    mark_running(&db, "work", "turn-done", 9).unwrap();
    backend.turns.lock().await.insert(
        "thread-a".into(),
        BTreeMap::from([("turn-done".into(), TurnStatus::Completed)]),
    );
    let reads_before = backend.reads.lock().await.len();

    let reconciled = queue.recover().await.unwrap();

    assert_eq!(backend.reads.lock().await.len(), reads_before + 1);
    assert!(reconciled.read_unavailable_targets.is_empty());
    assert_eq!(reconciled.unresolved, 1);
    assert_eq!(list(&db).unwrap()[0].state, QueueJobState::Running);
}

fn job<'a>(id: &'a str, target: &'a str, generation: i64, created_at: f64) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 71,
        owner_user_id: Some(10),
        discord_message_id: None,
        app_server_generation: generation,
        prompt: id,
        queued: true,
        ack_sent: true,
        created_at,
    }
}
