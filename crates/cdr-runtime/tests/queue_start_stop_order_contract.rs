use std::{path::PathBuf, sync::Arc, time::Duration};

use cdr_app_server::{DeadGenerationFence, RequestId, ResidentAppServer, requests::AppRequest};
use cdr_runtime::{
    app_backend::AppServerTurnBackend, dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_store::queue::{self, QueueJobState};
use rusqlite::Connection;
use serde_json::json;

#[path = "support/queue_start_stop_order_fence.rs"]
mod ordering;
#[path = "support/action_app_server.rs"]
mod server_support;
use ordering::{Action, Captured, OrderedFence, Race, Timing};

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    log: PathBuf,
    server: Arc<ResidentAppServer>,
    queue: QueueCoordinator<AppServerTurnBackend>,
    fence: Arc<OrderedFence>,
}

impl Fixture {
    async fn new(race: Race) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("queue.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let inner = RuntimeDeadGenerationFence::new(db.clone(), "start-order-runtime".into(), None)
            .unwrap();
        let fence = Arc::new(OrderedFence::new(inner, db.clone(), race));
        let mut config = native_fixture::config("action");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        let server = Arc::new(
            ResidentAppServer::start_with_dead_generation_fence(config, fence.clone())
                .await
                .unwrap(),
        );
        let queue = QueueCoordinator::new(
            db.clone(),
            Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
        );
        Self {
            _temp: temp,
            db,
            log,
            server,
            queue,
            fence,
        }
    }

    fn starts(&self, target: &str) -> usize {
        server_support::rpc_log(&self.log)
            .iter()
            .filter(|r| r["method"] == "turn/start" && r["params"]["threadId"] == target)
            .count()
    }

    fn assert_stored(&self, race: Race, original: &Captured, sent: bool) {
        let cancelled = matches!(race.action, Action::Recovery | Action::StopThenRecovery);
        let jobs = queue::list_filtered(&self.db, Some("thread-b"), None).unwrap();
        if cancelled {
            assert!(
                jobs.is_empty(),
                "recovery removed only the original captured job"
            );
        } else {
            assert_eq!(jobs.len(), 1);
            let job = &jobs[0];
            assert_eq!(job.job_id, original.job.job_id);
            assert_eq!(job.prompt, original.job.prompt);
            assert_eq!(job.target_thread_id, original.job.target_thread_id);
            assert_eq!(job.channel_id, original.job.channel_id);
            assert_eq!(job.owner_user_id, original.job.owner_user_id);
            assert_eq!(job.discord_message_id, original.job.discord_message_id);
            assert_eq!(job.attempt_count, 1, "no second automatic attempt");
            if sent {
                assert_eq!(job.state, QueueJobState::Running);
                assert_eq!(job.turn_id.as_deref(), Some("existing-turn"));
            }
        }
        let hold = cdr_store::execution_hold::reason(&self.db, &original.job.job_id).unwrap();
        assert_eq!(hold.is_some(), race.action != Action::RecoveryRollback);
        let cancelled_count: i64 = Connection::open(&self.db)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM codex_request_cancellations WHERE job_id=?",
                [&original.job.job_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cancelled_count, i64::from(cancelled));
        let recorded = ordering::journal(&self.db);
        if sent {
            let mut expected = original.prepared.clone().unwrap();
            expected.state = "reply_ok".into();
            assert_eq!(
                recorded,
                Some(expected),
                "exact admission identity retained after real reply"
            );
        } else {
            assert!(
                recorded.is_none(),
                "stop/recovery won before mutation admission"
            );
            assert!(original.prepared.is_none());
        }
    }

    async fn finish(&self, original: &Captured, sent: bool) {
        for _ in 0..2 {
            let _ = self.queue.kick_target("thread-b").await;
            let _ = self.queue.recover_target("thread-b").await;
        }
        // Even a fresh mutation occurrence cannot reuse the original queue claim.
        assert!(
            self.fence
                .begin_queue_mutation(
                    (self.server.instance_id(), self.server.generation()),
                    ("forbidden-replay", &RequestId::Integer(999)),
                    "turn/start",
                    &original.params,
                    &original.claim,
                )
                .is_err()
        );
        let independent = tokio::time::timeout(
            Duration::from_secs(5),
            self.queue
                .submit("thread-c", 43, 4, Some(802), "independent B"),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(independent.turn_id.is_some());
        // Real request/reply observation barrier, not a sleep or synthetic terminal.
        self.server
            .execute(
                AppRequest {
                    method: "thread/read",
                    params: json!({"threadId":"thread-c"}),
                    timeout: Duration::from_secs(2),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(self.starts("thread-b"), usize::from(sent));
        assert_eq!(self.starts("thread-c"), 1);
        assert!(!self.server.lifecycle_snapshot().await.quarantined);
        tokio::time::timeout(Duration::from_secs(3), self.server.close())
            .await
            .unwrap()
            .unwrap();
    }
}

async fn exercise(race: Race) {
    let f = Fixture::new(race).await;
    let sent = race.timing == Timing::After || race.action == Action::RecoveryRollback;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        f.queue.submit("thread-b", 42, 3, Some(801), "original A"),
    )
    .await
    .unwrap();
    let original = f
        .fence
        .captured
        .lock()
        .unwrap()
        .clone()
        .expect("real queue writer hook must run");
    if sent && !matches!(race.action, Action::Recovery | Action::StopThenRecovery) {
        assert!(result.unwrap().turn_id.is_some());
    }
    assert_eq!(f.starts("thread-b"), usize::from(sent));
    f.assert_stored(race, &original, sent);
    f.finish(&original, sent).await;
    let retained = ordering::journal(&f.db);
    assert_eq!(
        retained.is_some(),
        sent,
        "normal close/reopen cannot erase admission history"
    );
    // This is a reopened store check, not an OS crash/replacement-worker simulation.
}

#[tokio::test]
async fn stop_before_queue_writer_commits_sends_no_start() {
    exercise(Race {
        timing: Timing::Before,
        action: Action::Stop,
    })
    .await;
}
#[tokio::test]
async fn queue_writer_commit_before_stop_preserves_one_start() {
    exercise(Race {
        timing: Timing::After,
        action: Action::Stop,
    })
    .await;
}
#[tokio::test]
async fn recovery_before_queue_writer_commits_sends_no_start() {
    exercise(Race {
        timing: Timing::Before,
        action: Action::Recovery,
    })
    .await;
}
#[tokio::test]
async fn queue_writer_commit_before_recovery_preserves_one_start() {
    exercise(Race {
        timing: Timing::After,
        action: Action::Recovery,
    })
    .await;
}
#[tokio::test]
async fn stop_then_recovery_before_queue_writer_sends_no_start() {
    exercise(Race {
        timing: Timing::Before,
        action: Action::StopThenRecovery,
    })
    .await;
}
#[tokio::test]
async fn queue_writer_before_stop_then_recovery_is_not_replayed() {
    exercise(Race {
        timing: Timing::After,
        action: Action::StopThenRecovery,
    })
    .await;
}
#[tokio::test]
async fn failed_recovery_before_admission_rolls_back_and_allows_original_start() {
    exercise(Race {
        timing: Timing::Before,
        action: Action::RecoveryRollback,
    })
    .await;
}
#[tokio::test]
async fn failed_recovery_after_admission_rolls_back_without_a_second_start() {
    exercise(Race {
        timing: Timing::After,
        action: Action::RecoveryRollback,
    })
    .await;
}
