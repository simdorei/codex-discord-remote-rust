use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::queue::{
    NewQueueJob, QueueJobState, begin_attempt, enqueue, list, mark_running, record_start_failure,
};
use rusqlite::Connection;
use tokio::sync::Mutex;

#[derive(Default)]
struct RecoveryBackend {
    active: Mutex<Option<String>>,
    loaded: AtomicBool,
    require_resume: AtomicBool,
    resume_failure: AtomicBool,
    resumes: Mutex<Vec<String>>,
    turns: Mutex<BTreeMap<String, TurnStatus>>,
    starts: Mutex<Vec<String>>,
}

impl TurnBackend for RecoveryBackend {
    fn generation(&self) -> u64 {
        7
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(self.active.lock().await.clone()) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async {
            if self.require_resume.load(Ordering::SeqCst) && !self.loaded.load(Ordering::SeqCst) {
                return Err(cdr_runtime::queue_runner::BackendFailure::definite(
                    "thread not loaded",
                ));
            }
            Ok(self
                .turns
                .lock()
                .await
                .iter()
                .map(|(id, status)| TurnRecord {
                    turn_id: id.clone(),
                    status: *status,
                })
                .collect())
        })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.resumes.lock().await.push(thread_id.into());
            if self.resume_failure.load(Ordering::SeqCst) {
                return Err(BackendFailure::definite(
                    "thread already has an active writer",
                ));
            }
            self.loaded.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts.lock().await.push(prompt.into());
            let id = format!("fresh-{}", self.starts.lock().await.len());
            self.turns
                .lock()
                .await
                .insert(id.clone(), TurnStatus::InProgress);
            *self.active.lock().await = Some(id.clone());
            Ok(id)
        })
    }
}

#[tokio::test]
async fn running_read_failure_does_not_attempt_writer_resume() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("job-a", 1, 1, "recover after restart")).unwrap();
    begin_attempt(&db, "job-a", &[], 1).unwrap();
    mark_running(&db, "job-a", "turn-a", 1).unwrap();
    let backend = Arc::new(RecoveryBackend::default());
    backend.require_resume.store(true, Ordering::SeqCst);
    backend
        .turns
        .lock()
        .await
        .insert("turn-a".into(), TurnStatus::InProgress);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = coordinator.recover().await.unwrap();

    assert_eq!(
        report.read_unavailable_targets,
        std::collections::BTreeSet::from(["thread-a".into()])
    );
    assert!(backend.resumes.lock().await.is_empty());
    assert_eq!(list(&db).unwrap()[0].state, QueueJobState::Running);
}

#[tokio::test]
async fn transient_resume_conflict_keeps_jobs_unresolved_without_aborting_runtime_startup() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("job-a", 2, 1, "keep durable")).unwrap();
    let backend = Arc::new(RecoveryBackend::default());
    backend.resume_failure.store(true, Ordering::SeqCst);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = coordinator.recover().await.unwrap();

    assert_eq!(report.unresolved, 1);
    assert!(report.mutation_unavailable_targets.contains("thread-a"));
    assert_eq!(list(&db).unwrap()[0].state, QueueJobState::Pending);
}

fn job<'a>(id: &'a str, message: i64, generation: i64, prompt: &'a str) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: "thread-a",
        channel_id: 10,
        owner_user_id: Some(20),
        discord_message_id: Some(message),
        app_server_generation: generation,
        prompt,
        queued: true,
        ack_sent: true,
        created_at: f64::from(i32::try_from(message).unwrap()),
    }
}

#[tokio::test]
async fn old_ambiguous_start_adopts_the_single_new_turn_without_resending() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("job-a", 1, 1, "do once")).unwrap();
    begin_attempt(&db, "job-a", &["old".into()], 1).unwrap();
    expire_starting(&db, "job-a");
    let backend = Arc::new(RecoveryBackend::default());
    backend.turns.lock().await.extend([
        ("old".into(), TurnStatus::Completed),
        ("recovered".into(), TurnStatus::InProgress),
    ]);
    *backend.active.lock().await = Some("recovered".into());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = coordinator.recover().await.unwrap();

    assert_eq!(report.recovered_running, 1);
    assert_eq!(report.started, 0);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(jobs[0].turn_id.as_deref(), Some("recovered"));
    // Recover the exact turn, not a new execution owner (revision 10 contract).
    assert_eq!(jobs[0].app_server_generation, 1);
    assert_eq!(jobs[0].execution_generation, Some(1));
    assert_eq!(jobs[0].attempt_count, 1);
    assert_eq!(jobs[0].baseline_turn_ids, vec!["old".to_owned()]);
    assert_eq!(report.adopted, 0);
    assert!(backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn old_generation_without_a_new_turn_stays_unknown_after_lease_and_backoff() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("job-a", 2, 1, "do not replay after restart")).unwrap();
    begin_attempt(&db, "job-a", &[], 1).unwrap();
    let backend = Arc::new(RecoveryBackend::default());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    for _ in 0..3 {
        expire_starting(&db, "job-a");
        let report = coordinator.recover().await.unwrap();
        assert_eq!(report.requeued, 0);
        assert_eq!(report.started, 0);
        assert_eq!(report.unresolved, 1);
        let job = &list(&db).unwrap()[0];
        assert_eq!(job.state, QueueJobState::Starting);
        assert_eq!(job.attempt_count, 1);
        assert_eq!(job.app_server_generation, 1);
        assert!(job.turn_id.is_none());
        assert!(job.last_error.contains("does not authorize retry"));
    }
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
}

#[tokio::test]
async fn reused_generation_empty_read_preserves_the_original_ambiguous_error() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("job-a", 3, 7, "preserve unknown after restart")).unwrap();
    begin_attempt(&db, "job-a", &[], 7).unwrap();
    record_start_failure(&db, "job-a", 7, "original response was lost", true).unwrap();
    let backend = Arc::new(RecoveryBackend::default());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    for _ in 0..3 {
        expire_starting(&db, "job-a");
        let report = coordinator.recover().await.unwrap();
        assert_eq!(report.requeued, 0);
        assert_eq!(report.started, 0);
        assert_eq!(report.unresolved, 1);
        let job = &list(&db).unwrap()[0];
        assert_eq!(job.state, QueueJobState::Starting);
        assert_eq!(job.attempt_count, 1);
        assert_eq!(job.app_server_generation, 7);
        assert_eq!(job.last_error, "original response was lost");
    }
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
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

#[tokio::test]
async fn completed_running_job_is_preserved_for_durable_final_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("done", 4, 1, "old")).unwrap();
    begin_attempt(&db, "done", &[], 1).unwrap();
    mark_running(&db, "done", "turn-done", 1).unwrap();
    enqueue(&db, job("next", 5, 1, "next")).unwrap();
    let backend = Arc::new(RecoveryBackend::default());
    backend
        .turns
        .lock()
        .await
        .insert("turn-done".into(), TurnStatus::Completed);
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = coordinator.recover().await.unwrap();

    assert_eq!(report.completed, 0);
    assert_eq!(report.unresolved, 1);
    assert_eq!(report.started, 0);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].job_id, "done");
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert!(backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn recorded_definite_start_failure_remains_retryable_after_cold_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("definite", 5, 1, "confirmed safe retry")).unwrap();
    begin_attempt(&db, "definite", &[], 1).unwrap();
    record_start_failure(&db, "definite", 1, "request definitely not sent", false).unwrap();
    expire_starting(&db, "definite");
    let backend = Arc::new(RecoveryBackend::default());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = coordinator.recover().await.unwrap();

    assert_eq!(report.requeued, 0);
    assert_eq!(report.started, 1);
    assert_eq!(*backend.starts.lock().await, vec!["confirmed safe retry"]);
    let job = &list(&db).unwrap()[0];
    assert_eq!(job.state, QueueJobState::Running);
    assert_eq!(job.attempt_count, 2);
    assert_eq!(job.app_server_generation, 7);
}

#[tokio::test]
async fn failed_unknown_recovery_write_preserves_starting_and_never_authorizes_retry() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("unknown", 6, 1, "do not replay")).unwrap();
    begin_attempt(&db, "unknown", &[], 1).unwrap();
    expire_starting(&db, "unknown");
    let connection = Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_recovery_record BEFORE UPDATE OF last_error ON codex_turn_queue \
         BEGIN SELECT RAISE(ABORT,'injected recovery record failure'); END;",
        )
        .unwrap();
    let backend = Arc::new(RecoveryBackend::default());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    assert!(coordinator.recover().await.is_err());
    let job = &list(&db).unwrap()[0];
    assert_eq!(job.state, QueueJobState::Starting);
    assert_eq!(job.attempt_count, 1);
    assert_eq!(job.app_server_generation, 1);
    assert_eq!(job.updated_at.to_bits(), 0.0_f64.to_bits());
    assert!(job.last_error.is_empty());
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
    connection
        .execute_batch("DROP TRIGGER reject_recovery_record;")
        .unwrap();
    for _ in 0..3 {
        expire_starting(&db, "unknown");
        let report = coordinator.recover().await.unwrap();
        assert_eq!(report.requeued, 0);
        assert_eq!(report.started, 0);
        assert_eq!(report.unresolved, 1);
    }
    let job = &list(&db).unwrap()[0];
    assert_eq!(job.state, QueueJobState::Starting);
    assert_eq!(job.attempt_count, 1);
    assert!(job.last_error.contains("does not authorize retry"));
    assert!(backend.starts.lock().await.is_empty());
}
