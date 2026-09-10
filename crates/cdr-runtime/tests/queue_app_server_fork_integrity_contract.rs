use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cdr_runtime::queue_runner::{
    BoxBackendFuture, QueueCoordinator, QueueRunnerError, TurnBackend, TurnRecord,
};
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{AppServerForkHandoffError, NewQueueJob, QueueJobState, enqueue, list};

#[derive(Default)]
struct IntegrityBackend {
    forks: AtomicUsize,
    starts: AtomicUsize,
}

impl TurnBackend for IntegrityBackend {
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

    fn fork_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.forks.fetch_add(1, Ordering::SeqCst);
            Ok("must-not-fork".into())
        })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok("must-not-start".into())
        })
    }
}

#[tokio::test]
async fn duplicated_discord_mapping_stops_recovery_before_fork_or_turn_start() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Source", 70, 71, 1.0).unwrap();
    upsert_thread(&db, "duplicate", "project", "Duplicate", 70, 71, 2.0).unwrap();
    enqueue(
        &db,
        NewQueueJob {
            job_id: "pending",
            target_thread_id: "source",
            channel_id: 71,
            owner_user_id: Some(10),
            discord_message_id: Some(101),
            app_server_generation: 9,
            prompt: "do not deliver through ambiguous routing",
            queued: true,
            ack_sent: true,
            created_at: 3.0,
        },
    )
    .unwrap();
    let backend = Arc::new(IntegrityBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let error = queue.recover().await.unwrap_err();

    assert!(matches!(
        error,
        QueueRunnerError::ForkHandoff(AppServerForkHandoffError::MissingOrStaleMapping { .. })
    ));
    assert_eq!(backend.forks.load(Ordering::SeqCst), 0);
    assert_eq!(backend.starts.load(Ordering::SeqCst), 0);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs[0].state, QueueJobState::Pending);
}
