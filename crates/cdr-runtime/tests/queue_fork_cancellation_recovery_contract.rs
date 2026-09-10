use std::collections::VecDeque;
use std::sync::Arc;

use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, QueueRunnerError, TurnBackend, TurnRecord,
};
use cdr_store::delivery::list_pending;
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    NewQueueJob, QueueJobState, UNRESOLVED_FORK_ERROR_PREFIX, enqueue, list,
    unresolved_app_server_fork_handoff_for_source,
};
use rusqlite::Connection;
use tokio::sync::Mutex;

#[derive(Default)]
struct CancellationFailureBackend {
    forks: Mutex<Vec<String>>,
    fork_results: Mutex<VecDeque<Result<String, BackendFailure>>>,
}

impl TurnBackend for CancellationFailureBackend {
    fn generation(&self) -> u64 {
        9
    }

    fn requires_app_server_fork(&self) -> bool {
        true
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn fork_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.forks.lock().await.push(thread_id.to_owned());
            self.fork_results
                .lock()
                .await
                .pop_front()
                .expect("the test supplies one fork result")
        })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async { Err(BackendFailure::definite("unexpected turn/start")) })
    }
}

#[tokio::test]
async fn atomic_definite_record_and_cancel_failure_rolls_back_every_partial_write() {
    let (_temp, db, backend, queue) = fixture();
    install_cancel_failure_trigger(&db);

    let error = queue.recover().await.unwrap_err();

    let (source, handoff, failure, recording) = match error {
        QueueRunnerError::ForkFailureRecording {
            source_thread_id,
            handoff_id,
            failure,
            recording,
        } => (source_thread_id, handoff_id, failure, recording),
        error => panic!("unexpected recovery error: {error}"),
    };
    assert_eq!(source, "source");
    assert!(!handoff.is_empty());
    assert!(failure.message.contains("thread/fork rejected"));
    assert!(recording.to_string().contains("injected cancel failure"));
    assert_eq!(*backend.forks.lock().await, vec!["source"]);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, QueueJobState::Pending);
    assert_eq!(jobs[0].target_thread_id, "source");
    assert_eq!(jobs[0].prompt, "preserve exactly once");
    assert_eq!(jobs[0].attempt_count, 0);
    assert_eq!(jobs[0].turn_id, None);
    assert!(jobs[0].last_error.is_empty());
    let handoff = unresolved_app_server_fork_handoff_for_source(&db, "source")
        .unwrap()
        .expect("the pre-RPC intent remains after the atomic transaction rolls back");
    assert!(!handoff.fork_failure_ambiguous);
    assert!(handoff.last_fork_error.is_empty());
    assert!(list_pending(&db).unwrap().is_empty());
}

#[tokio::test]
async fn recovery_after_atomic_store_failure_fences_without_a_second_fork_or_duplicate_warning() {
    let (_temp, db, backend, queue) = fixture();
    install_cancel_failure_trigger(&db);

    let first = queue.recover().await.unwrap_err();
    assert!(matches!(
        first,
        QueueRunnerError::ForkFailureRecording { .. }
    ));
    let second = queue.recover().await.unwrap();
    let third = queue.recover().await.unwrap();

    assert!(second.unavailable_targets.contains("source"));
    assert!(third.unavailable_targets.contains("source"));
    assert_eq!(*backend.forks.lock().await, vec!["source"]);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, QueueJobState::Pending);
    assert_eq!(jobs[0].target_thread_id, "source");
    assert_eq!(jobs[0].prompt, "preserve exactly once");
    assert_eq!(jobs[0].attempt_count, 0);
    assert_eq!(jobs[0].turn_id, None);
    assert!(jobs[0].last_error.starts_with(UNRESOLVED_FORK_ERROR_PREFIX));
    let handoff = unresolved_app_server_fork_handoff_for_source(&db, "source")
        .unwrap()
        .expect("unknown fork outcome must remain fenced");
    assert!(handoff.fork_failure_ambiguous);
    assert!(
        handoff
            .last_fork_error
            .contains("interrupted before a response")
    );
    let notices = list_pending(&db).unwrap();
    assert_eq!(notices.len(), 1, "recovery refresh must remain idempotent");
    assert!(notices[0].content.contains("interrupted before a response"));
}

fn fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Arc<CancellationFailureBackend>,
    QueueCoordinator<CancellationFailureBackend>,
) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 70, 71, 1.0).unwrap();
    enqueue(
        &db,
        NewQueueJob {
            job_id: "pending",
            target_thread_id: "source",
            channel_id: 70,
            owner_user_id: Some(10),
            discord_message_id: Some(72),
            app_server_generation: 9,
            prompt: "preserve exactly once",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    unresolved_app_server_fork_handoff_for_source(&db, "source").unwrap();
    let backend = Arc::new(CancellationFailureBackend {
        forks: Mutex::default(),
        fork_results: Mutex::new(VecDeque::from([Err(BackendFailure::definite(
            "thread/fork rejected",
        ))])),
    });
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    (temp, db, backend, queue)
}

fn install_cancel_failure_trigger(db: &std::path::Path) {
    Connection::open(db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_fork_handoff_cancel \
             BEFORE DELETE ON codex_thread_fork_handoffs \
             BEGIN SELECT RAISE(ABORT, 'injected cancel failure'); END;",
        )
        .unwrap();
}
