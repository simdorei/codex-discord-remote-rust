use std::{path::PathBuf, sync::Arc, time::Duration};

use cdr_app_server::{AppServerError, ResidentAppServer, ServerRequest};
use cdr_runtime::{dead_generation_recovery::RuntimeDeadGenerationFence, soak::native_fixture};
use cdr_store::{ingress::stop::control, mutation_attempt::response, queue};
use serde_json::{Value, json};

#[path = "support/response_stop_order_fence.rs"]
mod fence;
use fence::{Action, OrderedFence, Race, Timing};

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    log: PathBuf,
    server: ResidentAppServer,
    fence: Arc<OrderedFence>,
    a: ServerRequest,
    b: ServerRequest,
    original: queue::StoredQueueJob,
}

impl Fixture {
    async fn new(race: Race) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let inner =
            RuntimeDeadGenerationFence::new(db.clone(), "response-order".into(), None).unwrap();
        let fence = Arc::new(OrderedFence::new(inner, db.clone(), race));
        let mut config = native_fixture::config("approval");
        config.environment.insert(
            "CDR_APPROVAL_TEST_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        let server = ResidentAppServer::start_with_dead_generation_fence(config, fence.clone())
            .await
            .unwrap();
        let generation = i64::try_from(server.generation()).unwrap();
        for (target, turn, job, channel) in [
            ("thread-b", "turn-b", "original-a", 42),
            ("independent-b", "independent-turn", "original-b", 43),
        ] {
            cdr_store::mapping::upsert_thread(&db, target, "project", "fixture", 100, channel, 1.0)
                .unwrap();
            queue::enqueue(
                &db,
                queue::NewQueueJob {
                    job_id: job,
                    target_thread_id: target,
                    channel_id: channel,
                    owner_user_id: Some(3),
                    discord_message_id: Some(channel + 100),
                    app_server_generation: generation,
                    prompt: "original fixture request",
                    queued: false,
                    ack_sent: true,
                    created_at: 1.0,
                },
            )
            .unwrap();
            queue::begin_attempt(&db, job, &[], generation).unwrap();
            queue::mark_running(&db, job, turn, generation).unwrap();
        }
        let original = queue::list_filtered(&db, Some("thread-b"), None)
            .unwrap()
            .remove(0);
        let a = pending(&server, "thread-b", "turn-b", "reply-a").await;
        let b = pending(&server, "independent-b", "independent-turn", "reply-b").await;
        Self {
            _temp: temp,
            db,
            log,
            server,
            fence,
            a,
            b,
            original,
        }
    }

    async fn reply(&self) -> Result<(), AppServerError> {
        self.server
            .respond_current(
                &self.a.id,
                self.a.occurrence,
                json!({"decision":"accept"}),
                self.server.generation(),
            )
            .await
    }

    async fn independent_progress(&self) {
        self.server
            .respond_current(
                &self.b.id,
                self.b.occurrence,
                json!({"decision":"accept"}),
                self.server.generation(),
            )
            .await
            .unwrap();
        // A known observational RPC provides a real post-frame consumption barrier.
        // The synthetic approval child rejects this method, which is the expected reply.
        let error = self
            .server
            .request(
                "thread/read",
                json!({"threadId":"independent-b"}),
                Duration::from_secs(2),
                Some(self.server.generation()),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AppServerError::Remote { code: -32_601, .. }
        ));
    }

    fn assert_stored(&self, race: Race, sent: bool) {
        let db = rusqlite::Connection::open(&self.db).unwrap();
        let cancelled: i64 = db
            .query_row(
                "SELECT count(*) FROM codex_request_cancellations WHERE job_id='original-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let recovery = matches!(race.action, Action::Recovery | Action::StopThenRecovery);
        assert_eq!(cancelled, i64::from(recovery));
        let jobs = queue::list_filtered(&self.db, Some("thread-b"), None).unwrap();
        if recovery {
            assert!(jobs.is_empty());
        } else {
            assert_eq!(jobs, vec![self.original.clone()]);
        }
        let stop = self.fence.original_stop.lock().unwrap().clone();
        let expected_stop = matches!(race.action, Action::Stop | Action::StopThenRecovery);
        assert_eq!(stop.is_some(), expected_stop);
        assert_eq!(
            control::target_is_held(&self.db, "thread-b").unwrap(),
            expected_stop
        );
        if let Some(stop) = stop {
            assert_eq!(
                control::phase(&self.db, &stop.operation_id)
                    .unwrap()
                    .as_deref(),
                Some("accepted")
            );
            let saved: String = db
                .query_row(
                    "SELECT record_json FROM cdr_stop_controls WHERE operation_id=?",
                    [&stop.operation_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<control::StopControl>(&saved).unwrap(),
                stop
            );
        }
        assert_eq!(
            cdr_store::execution_hold::reason(&self.db, "original-a")
                .unwrap()
                .is_some(),
            race.action != Action::RecoveryRollback
        );
        let phases = db
            .prepare("SELECT phase FROM cdr_server_responses WHERE target_thread_id='thread-b'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let expected = if sent {
            vec![if race.fail_finish {
                "admitted".to_string()
            } else {
                "flushed".to_string()
            }]
        } else {
            vec![]
        };
        assert_eq!(
            phases, expected,
            "flush is not terminal and failed finish retains admission"
        );
    }

    async fn finish(self, race: Race, sent: bool) {
        self.assert_stored(race, sent);
        let healthy = !self.server.lifecycle_snapshot().await.quarantined;
        tokio::time::timeout(Duration::from_secs(3), self.server.close())
            .await
            .unwrap()
            .unwrap();
        self.assert_stored(race, sent);
        let rows = std::fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let count = |id: &str| {
            rows.iter()
                .filter(|r| {
                    r["id"] == id && (r.get("result").is_some() || r.get("error").is_some())
                })
                .count()
        };
        assert_eq!(
            count("reply-a"),
            usize::from(sent),
            "original A is written at most once, only if admission won"
        );
        assert_eq!(
            count("reply-b"),
            1,
            "B progresses on the same healthy resident"
        );
        assert!(
            healthy,
            "stop/cancellation/store failure is not shared I/O damage"
        );
        if race.action != Action::RecoveryRollback {
            let authority = self
                .fence
                .original_authority
                .lock()
                .unwrap()
                .clone()
                .unwrap();
            let request = json!({"id":self.a.id,"occurrence":self.a.occurrence,
                "method":self.a.method,"params":self.a.params});
            let scope = response::Scope {
                runtime: "response-order",
                resident: self.server.instance_id(),
                generation: i64::try_from(self.server.generation()).unwrap(),
                request: &request,
            };
            assert!(
                response::begin(
                    &self.db,
                    &scope,
                    &authority,
                    &json!({"id":self.a.id,"result":{"decision":"accept"}})
                )
                .is_err(),
                "reopened original authority never grants replay"
            );
        }
    }
}

async fn pending(server: &ResidentAppServer, target: &str, turn: &str, id: &str) -> ServerRequest {
    server
        .request(
            "test/pending",
            json!({"threadId":target,"turnId":turn,"requestId":id}),
            Duration::from_secs(2),
            Some(server.generation()),
        )
        .await
        .unwrap();
    server
        .pending_server_requests(Some(target))
        .await
        .unwrap()
        .into_iter()
        .find(|request| request.id == cdr_app_server::RequestId::String(id.into()))
        .unwrap()
}

async fn exercise(race: Race) {
    let f = Fixture::new(race).await;
    let sent = race.timing == Timing::After || race.action == Action::RecoveryRollback;
    let result = tokio::time::timeout(Duration::from_secs(3), f.reply())
        .await
        .unwrap();
    if !sent {
        assert!(matches!(result, Err(AppServerError::MutationHeld { .. })));
    } else if race.fail_finish {
        assert!(matches!(
            result,
            Err(AppServerError::MutationOutcomeUnknown { .. })
        ));
    } else {
        result.unwrap();
    }
    assert!(
        f.reply().await.is_err(),
        "a second click never reuses the original occurrence"
    );
    tokio::time::timeout(Duration::from_secs(5), f.independent_progress())
        .await
        .unwrap();
    f.finish(race, sent).await;
}

#[tokio::test]
async fn stop_commit_before_writer_admission_sends_no_original_response() {
    exercise(Race {
        timing: Timing::Before,
        action: Action::Stop,
        fail_finish: false,
    })
    .await;
}

#[tokio::test]
async fn writer_admission_before_stop_preserves_consumed_original_and_hold() {
    exercise(Race {
        timing: Timing::After,
        action: Action::Stop,
        fail_finish: false,
    })
    .await;
}

#[tokio::test]
async fn recovery_commit_before_writer_admission_sends_no_original_response() {
    exercise(Race {
        timing: Timing::Before,
        action: Action::Recovery,
        fail_finish: false,
    })
    .await;
}

#[tokio::test]
async fn writer_admission_before_recovery_never_rebinds_deleted_original() {
    exercise(Race {
        timing: Timing::After,
        action: Action::Recovery,
        fail_finish: false,
    })
    .await;
}

#[tokio::test]
async fn stop_then_recovery_before_admission_preserves_both_receipts() {
    exercise(Race {
        timing: Timing::Before,
        action: Action::StopThenRecovery,
        fail_finish: false,
    })
    .await;
}

#[tokio::test]
async fn writer_first_then_stop_and_recovery_preserves_both_receipts() {
    exercise(Race {
        timing: Timing::After,
        action: Action::StopThenRecovery,
        fail_finish: false,
    })
    .await;
}

#[tokio::test]
async fn recovery_rollback_before_admission_does_not_invent_cancellation() {
    exercise(Race {
        timing: Timing::Before,
        action: Action::RecoveryRollback,
        fail_finish: false,
    })
    .await;
}

#[tokio::test]
async fn recovery_rollback_after_admission_does_not_invent_cancellation() {
    exercise(Race {
        timing: Timing::After,
        action: Action::RecoveryRollback,
        fail_finish: false,
    })
    .await;
}

#[tokio::test]
async fn writer_first_stop_and_lost_finish_keep_unknown_without_blocking_b() {
    exercise(Race {
        timing: Timing::After,
        action: Action::Stop,
        fail_finish: true,
    })
    .await;
}
