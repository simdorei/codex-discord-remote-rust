use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use cdr_store::mapping::{thread_channels, upsert_thread};
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, QueueJobState, begin_app_server_fork_handoff,
    begin_attempt, complete_app_server_fork_handoff, enqueue, list,
};
use rusqlite::Connection;

struct LeaseBackend {
    db: Option<PathBuf>,
    handoff_done: AtomicBool,
    reads: AtomicUsize,
}

impl TurnBackend for LeaseBackend {
    fn generation(&self) -> u64 {
        5
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if let Some(db) = &self.db
                && !self.handoff_done.swap(true, Ordering::SeqCst)
            {
                begin_app_server_fork_handoff(
                    db,
                    NewAppServerForkHandoff {
                        handoff_id: "racing-handoff",
                        ambiguous_job_id: Some("starting"),
                        source_thread_id: thread_id,
                        expected_generation: 5,
                        quarantine_reason: "test concurrent ownership handoff",
                    },
                )
                .unwrap();
                complete_app_server_fork_handoff(db, "racing-handoff", "forked", 5).unwrap();
            }
            Ok(vec![TurnRecord {
                turn_id: "observed-turn".into(),
                status: TurnStatus::InProgress,
            }])
        })
    }

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
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
async fn fresh_starting_lease_is_read_but_not_reconciled_or_restarted() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job()).unwrap();
    let claimed = begin_attempt(&db, "starting", &[], 5).unwrap();
    let backend = Arc::new(LeaseBackend {
        db: None,
        handoff_done: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let report = queue.recover_target("source").await.unwrap();

    assert_eq!(backend.reads.load(Ordering::SeqCst), 1);
    assert_eq!(report.recovered_running, 0);
    assert_eq!(report.unresolved, 1);
    assert_eq!(list(&db).unwrap(), vec![claimed]);
}

#[tokio::test]
async fn concurrent_handoff_quarantine_wins_over_stale_starting_observation() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Source", 70, 71, 1.0).unwrap();
    enqueue(&db, job()).unwrap();
    begin_attempt(&db, "starting", &[], 5).unwrap();
    Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET updated_at = 0 WHERE job_id = 'starting'",
            [],
        )
        .unwrap();
    let backend = Arc::new(LeaseBackend {
        db: Some(db.clone()),
        handoff_done: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let queue = QueueCoordinator::new(db.clone(), backend);

    let report = queue.recover_target("source").await.unwrap();

    assert_eq!(report.recovered_running, 0);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, QueueJobState::Quarantined);
    assert!(
        jobs[0]
            .turn_id
            .as_deref()
            .unwrap()
            .starts_with("cdr-quarantined:")
    );
    assert!(
        jobs[0]
            .last_error
            .starts_with("[cdr-rust:app-server-fork-quarantine:v1]")
    );
    assert_eq!(thread_channels(&db, "source").unwrap(), None);
    assert_eq!(thread_channels(&db, "forked").unwrap(), Some((70, 71)));
}

fn job() -> NewQueueJob<'static> {
    NewQueueJob {
        job_id: "starting",
        target_thread_id: "source",
        channel_id: 71,
        owner_user_id: Some(10),
        discord_message_id: None,
        app_server_generation: 5,
        prompt: "do once",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}
