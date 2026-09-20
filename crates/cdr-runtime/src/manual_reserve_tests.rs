//! Production backend/queue regressions over the offline native server.
use crate::test_support::message_fixture::MessageFixture;
use cdr_app_server::ResidentAppServer;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};

async fn fixture(temp: &tempfile::TempDir) -> MessageFixture {
    let mut config = crate::soak::native_fixture::config("reserve-auto");
    config.environment.insert(
        "RESERVE_TEST_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    MessageFixture::with_reserve_server(
        temp,
        Arc::new(twilight_http::Client::new("fixture-token".into())),
        server,
    )
}

fn frames(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("rpc.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn enqueue_old_generation(db: &Path, job_id: &str, created_at: f64) {
    cdr_store::queue::enqueue(
        db,
        cdr_store::queue::NewQueueJob {
            job_id,
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            app_server_generation: 97,
            prompt: job_id,
            queued: true,
            ack_sent: false,
            created_at,
        },
    )
    .unwrap();
}

#[tokio::test]
async fn retirement_held_pending_never_resumes_even_after_coordinator_reconstruction() {
    for error in ["", "ordinary transient failure"] {
        let temp = tempfile::tempdir().unwrap();
        let f = fixture(&temp).await;
        let db = f.queue.db_path();
        enqueue_old_generation(db, "held", 1.0);
        rusqlite::Connection::open(db)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET last_error=? WHERE job_id='held'",
                [error],
            )
            .unwrap();
        let before = cdr_store::queue::list(db).unwrap();
        assert_eq!(
            cdr_store::reserve_retirement::retire(db, &[])
                .unwrap()
                .held_jobs,
            1
        );
        assert_eq!(f.queue.recover().await.unwrap().started, 0);
        let reconstructed = crate::queue_runner::QueueCoordinator::new(
            db.to_path_buf(),
            Arc::new(crate::app_backend::AppServerTurnBackend::new(
                f.server.clone(),
            )),
        );
        assert_eq!(reconstructed.recover().await.unwrap().started, 0);
        assert_eq!(cdr_store::queue::list(db).unwrap(), before);
        f.server.close().await.unwrap();
        assert!(
            frames(temp.path())
                .iter()
                .all(|r| !matches!(r["method"].as_str(), Some("thread/resume" | "turn/start")))
        );
    }
}

#[tokio::test]
async fn held_head_backoff_cannot_delay_eligible_successor_after_generation_adoption() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp).await;
    let db = f.queue.db_path();
    enqueue_old_generation(db, "held", 1.0);
    let connection = rusqlite::Connection::open(db).unwrap();
    connection
        .execute(
            "UPDATE codex_turn_queue SET last_error='ordinary transient failure',
        attempt_count=6,updated_at=9000000000 WHERE job_id='held'",
            [],
        )
        .unwrap();
    assert_eq!(
        cdr_store::reserve_retirement::retire(db, &[])
            .unwrap()
            .held_jobs,
        1
    );
    // A new post-retirement request is subject to its own retry deadline.
    enqueue_old_generation(db, "eligible", 2.0);
    connection
        .execute(
            "UPDATE codex_turn_queue SET last_error='successor transient failure',
        attempt_count=1,updated_at=9000000000 WHERE job_id='eligible'",
            [],
        )
        .unwrap();
    assert_eq!(f.queue.recover().await.unwrap().started, 0);
    assert!(
        frames(temp.path())
            .iter()
            .all(|r| r["method"] != "thread/resume")
    );
    connection
        .execute(
            "UPDATE codex_turn_queue SET updated_at=1 WHERE job_id='eligible'",
            [],
        )
        .unwrap();
    let reconstructed = crate::queue_runner::QueueCoordinator::new(
        db.to_path_buf(),
        Arc::new(crate::app_backend::AppServerTurnBackend::new(
            f.server.clone(),
        )),
    );
    let report = reconstructed.recover().await.unwrap();
    assert_eq!(report.started, 1);
    assert!(report.adopted > 0);
    let jobs = cdr_store::queue::list(db).unwrap();
    let held = jobs.iter().find(|j| j.job_id == "held").unwrap();
    assert_eq!(held.state, cdr_store::queue::QueueJobState::Pending);
    assert_eq!(held.attempt_count, 6);
    assert_eq!(held.last_error, "ordinary transient failure");
    assert!(
        cdr_store::execution_hold::reason(db, "held")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        jobs.iter().find(|j| j.job_id == "eligible").unwrap().state,
        cdr_store::queue::QueueJobState::Running
    );
    assert_eq!(reconstructed.recover().await.unwrap().started, 0);
    f.server.close().await.unwrap();
    let log = frames(temp.path());
    assert_eq!(
        log.iter().filter(|r| r["method"] == "turn/start").count(),
        1
    );
}

#[tokio::test]
async fn ordinary_request_does_not_observe_or_change_reserve_settings() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp).await;
    let result = f
        .queue
        .submit_identified("manual-only", "thread-b", 42, 3, None, "input")
        .await
        .unwrap();
    f.server.close().await.unwrap();
    assert!(result.turn_id.is_some());
    let log = frames(temp.path());
    assert!(
        log.iter().all(|row| !matches!(
            row["method"].as_str(),
            Some("account/rateLimits/read" | "model/list" | "thread/settings/update")
        )),
        "ordinary admission must not invoke automatic Reserve RPCs"
    );
    let starts: Vec<_> = log
        .iter()
        .filter(|row| row["event"] == "start_settings")
        .collect();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["settings"]["model"], "model-a");
    assert!(
        cdr_store::reserve_policy::get(f.queue.db_path(), "thread-b")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn usage_limit_stays_failed_without_creating_a_policy_or_replaying() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp).await;
    f.server
        .request(
            "test/configure",
            json!({"threadId":"thread-b","ordinary":true,"reject_next_start":"usage"}),
            Duration::from_secs(4),
            None,
        )
        .await
        .unwrap();
    let result = f
        .queue
        .submit_identified("limited", "thread-b", 42, 3, None, "input")
        .await
        .unwrap();
    assert!(result.turn_id.is_none());
    assert!(result.warning.is_some());
    let _ = f.queue.recover().await.unwrap();
    assert!(!f.queue.busy_status("thread-b").await.unwrap().busy);
    f.queue.kick_target("thread-b").await.unwrap();
    f.server.close().await.unwrap();
    let log = frames(temp.path());
    assert_eq!(
        log.iter()
            .filter(|row| row["method"] == "turn/start")
            .count(),
        1
    );
    assert!(
        log.iter()
            .all(|row| row["method"] != "thread/settings/update")
    );
    assert!(
        cdr_store::reserve_policy::get(f.queue.db_path(), "thread-b")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn old_direct_and_bound_toggles_are_decoded_but_never_resolve_or_mutate_settings() {
    use crate::{
        action_executor::ActionContext,
        command_plan::{AUTO_RESERVE_REMOVED, CommandAction},
    };
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp).await;
    for enabled in [false, true] {
        let action: CommandAction = serde_json::from_value(
            json!({"AutoReserve":{"reference":"missing-thread","enabled":enabled}}),
        )
        .unwrap();
        let context = ActionContext {
            channel_id: 42,
            user_id: 3,
            discord_message_id: None,
            auto_queue_when_busy: false,
        };
        assert_eq!(
            f.executor
                .execute_with_context(action.clone(), context)
                .await
                .unwrap()
                .text,
            AUTO_RESERVE_REMOVED
        );
        let payload = json!({"settings_binding":{"target":"missing-thread","route":"Explicit","command":action}});
        let db = rusqlite::Connection::open(f.queue.db_path()).unwrap();
        db.execute("INSERT OR REPLACE INTO discord_ingress_journal
            (ingress_id,kind,channel_id,owner_user_id,payload_json,state,phase,target_thread_id,created_at,updated_at)
            VALUES ('old-toggle','message',42,3,?,'executing','settings','missing-thread',1,1)",[payload.to_string()]).unwrap();
        assert_eq!(
            f.executor
                .execute_with_ingress_context(action, context, "old-toggle")
                .await
                .unwrap()
                .text,
            AUTO_RESERVE_REMOVED
        );
        let after: String = db
            .query_row(
                "SELECT payload_json FROM discord_ingress_journal WHERE ingress_id='old-toggle'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, payload.to_string());
    }
    f.server.close().await.unwrap();
    assert!(frames(temp.path()).iter().all(|r| !matches!(
        r["method"].as_str(),
        Some("thread/resume" | "model/list" | "account/rateLimits/read" | "thread/settings/update")
    )));
}
