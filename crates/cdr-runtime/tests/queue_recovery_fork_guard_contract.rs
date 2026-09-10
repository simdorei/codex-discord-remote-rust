use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, QueueRunnerError, TurnBackend, TurnRecord,
};
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    AppServerForkHandoffError, NewAppServerForkHandoff, NewQueueJob, begin_app_server_fork_handoff,
    complete_app_server_fork_handoff, enqueue,
};

struct WriterConflictBackend {
    app_server_only: bool,
    fork_ambiguous: bool,
    forks: AtomicUsize,
}

impl TurnBackend for WriterConflictBackend {
    fn generation(&self) -> u64 {
        8
    }

    fn requires_app_server_fork(&self) -> bool {
        self.app_server_only
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            Err(BackendFailure::active_writer(format!(
                "thread/resume failed: thread {thread_id} already has an active writer"
            )))
        })
    }

    fn fork_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.forks.fetch_add(1, Ordering::SeqCst);
            if self.fork_ambiguous {
                return Err(BackendFailure::ambiguous("thread/fork response timed out"));
            }
            Ok("forked".into())
        })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async { Ok("must-not-start".into()) })
    }
}

#[tokio::test]
async fn non_app_server_backend_never_enters_writer_conflict_fork_path() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Source", 70, 71, 1.0).unwrap();
    enqueue(&db, job("pending", "source")).unwrap();
    let backend = Arc::new(WriterConflictBackend {
        app_server_only: false,
        fork_ambiguous: false,
        forks: AtomicUsize::new(0),
    });
    let queue = QueueCoordinator::new(db, Arc::clone(&backend));

    let report = queue.recover().await.unwrap();

    assert!(report.active_writer_targets.contains("source"));
    assert_eq!(backend.forks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn writer_conflict_mapping_error_without_durable_fence_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "initial", "project", "Initial", 80, 81, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "seed",
            ambiguous_job_id: None,
            source_thread_id: "initial",
            expected_generation: 8,
            quarantine_reason: "seed managed target",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&db, "seed", "managed", 8).unwrap();
    upsert_thread(&db, "duplicate", "project", "Duplicate", 80, 81, 2.0).unwrap();
    enqueue(&db, job("pending", "managed")).unwrap();
    let backend = Arc::new(WriterConflictBackend {
        app_server_only: true,
        fork_ambiguous: false,
        forks: AtomicUsize::new(0),
    });
    let queue = QueueCoordinator::new(db, backend);

    let error = queue.recover().await.unwrap_err();

    assert!(matches!(
        error,
        QueueRunnerError::ForkHandoff(AppServerForkHandoffError::MissingOrStaleMapping { .. })
    ));
}

#[tokio::test]
async fn ambiguous_writer_conflict_fork_is_nonfatal_only_after_durable_fencing() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "initial", "project", "Initial", 90, 91, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "seed-managed",
            ambiguous_job_id: None,
            source_thread_id: "initial",
            expected_generation: 8,
            quarantine_reason: "seed managed target",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&db, "seed-managed", "managed", 8).unwrap();
    enqueue(&db, job("pending", "managed")).unwrap();
    let backend = Arc::new(WriterConflictBackend {
        app_server_only: true,
        fork_ambiguous: true,
        forks: AtomicUsize::new(0),
    });
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = queue.recover().await.unwrap();

    assert!(report.active_writer_targets.contains("managed"));
    assert_eq!(backend.forks.load(Ordering::SeqCst), 1);
    assert!(
        cdr_store::queue::unresolved_app_server_fork_handoff_for_source(&db, "managed")
            .unwrap()
            .is_some()
    );
}

fn job<'a>(id: &'a str, target: &'a str) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 81,
        owner_user_id: Some(10),
        discord_message_id: None,
        app_server_generation: 8,
        prompt: id,
        queued: true,
        ack_sent: true,
        created_at: 3.0,
    }
}
