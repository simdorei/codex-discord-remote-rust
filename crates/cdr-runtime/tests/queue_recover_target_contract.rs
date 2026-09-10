use std::sync::Arc;

use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use cdr_store::queue::{NewQueueJob, QueueJobState, enqueue, list};
use tokio::sync::Mutex;

#[derive(Default)]
struct BoundedBackend {
    calls: Mutex<Vec<(String, String)>>,
}

impl TurnBackend for BoundedBackend {
    fn generation(&self) -> u64 {
        4
    }

    fn active_turn_id<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move {
            self.calls
                .lock()
                .await
                .push(("active".into(), thread_id.into()));
            Ok(None)
        })
    }

    fn read_turns<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async move {
            self.calls
                .lock()
                .await
                .push(("read".into(), thread_id.into()));
            Ok(Vec::new())
        })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.calls
                .lock()
                .await
                .push(("resume".into(), thread_id.into()));
            Ok(())
        })
    }

    fn start_turn<'a>(
        &'a self,
        thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.calls
                .lock()
                .await
                .push(("start".into(), thread_id.into()));
            Ok(format!("turn-{thread_id}"))
        })
    }
}

#[tokio::test]
async fn recover_target_only_reads_and_starts_the_requested_target() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("job-a", "thread-a", 1.0)).unwrap();
    enqueue(&db, job("job-b", "thread-b", 2.0)).unwrap();
    let backend = Arc::new(BoundedBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = queue.recover_target("thread-b").await.unwrap();

    assert_eq!(report.started, 1);
    assert!(
        backend
            .calls
            .lock()
            .await
            .iter()
            .all(|(_, target)| target == "thread-b")
    );
    let jobs = list(&db).unwrap();
    assert_eq!(jobs[0].target_thread_id, "thread-a");
    assert_eq!(jobs[0].state, QueueJobState::Pending);
    assert_eq!(jobs[1].target_thread_id, "thread-b");
    assert_eq!(jobs[1].state, QueueJobState::Running);
}

fn job<'a>(id: &'a str, target: &'a str, created_at: f64) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 70,
        owner_user_id: Some(10),
        discord_message_id: None,
        app_server_generation: 4,
        prompt: id,
        queued: true,
        ack_sent: true,
        created_at,
    }
}
