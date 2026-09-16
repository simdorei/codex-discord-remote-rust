//! DG2: a legitimate next-goal turn arrives while prior progress HTTP is pending.
//! Uses actual worker/transport/SQLite/Discord adapters; external app-server is a fixture.
use super::goal_handoff_tests::{make_worker, setup_running};
use super::*;
use crate::session_mirror_worker::{DiscordSessionMirrorSender, SessionMirrorWorker};
use crate::test_support::http_gate as http_boundary;
use cdr_store::{
    mapping::upsert_thread,
    mirror::{get_offset, update_cursor},
    queue,
};
use serde_json::json;

#[tokio::test]
async fn next_goal_terminal_during_progress_http_is_delivered_only_by_completion() {
    tokio::time::timeout(Duration::from_secs(10), scenario(false))
        .await
        .unwrap();
}

#[tokio::test]
async fn next_goal_terminal_before_handoff_commit_is_delivered_only_by_completion() {
    tokio::time::timeout(Duration::from_secs(10), scenario(true))
        .await
        .unwrap();
}

async fn prepare(
    temp: &tempfile::TempDir,
    address: String,
) -> (
    CompletionWorker,
    SessionMirrorWorker<DiscordSessionMirrorSender>,
) {
    let mut worker = make_worker(temp).await;
    worker.http = Arc::new(
        Client::builder()
            .token("test-token".into())
            .proxy(address, true)
            .ratelimiter(None)
            .timeout(Duration::from_secs(3))
            .build(),
    );
    setup_running(&worker);
    let db = worker.queue.db_path().to_owned();
    let rollout = temp.path().join("goal-rollout.jsonl");
    std::fs::write(&rollout, "").unwrap();
    let state = temp.path().join("state.sqlite");
    let conn = rusqlite::Connection::open(&state).unwrap();
    conn.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT);").unwrap();
    conn.execute("INSERT INTO threads VALUES ('thread','Title','C:/repo',1,?1,'gpt','high',0,0,0,'vscode','user')",[rollout.to_string_lossy().as_ref()]).unwrap();
    upsert_thread(&db, "thread", "project", "Title", 100, 42, 1.0).unwrap();
    update_cursor(&db, "thread", &rollout.to_string_lossy(), 0, 1.0).unwrap();
    let mirror = SessionMirrorWorker::new(
        state,
        db.clone(),
        Arc::new(DiscordSessionMirrorSender::new(
            Arc::clone(&worker.http),
            db.clone(),
        )),
    );
    (worker, mirror)
}

async fn request_goal_transition(worker: &CompletionWorker, method: &'static str) {
    worker
        .server
        .execute(
            cdr_app_server::requests::AppRequest {
                method,
                params: json!({}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
}

async fn scenario(early: bool) {
    let temp = tempfile::tempdir().unwrap();
    let gate = http_boundary::start().await;
    let (worker, mirror) = prepare(&temp, gate.address).await;
    let db = worker.queue.db_path().to_owned();
    let rollout = temp.path().join("goal-rollout.jsonl");
    let worker = Arc::new(worker);
    let mut notifications = worker.server.subscribe_notifications();
    if early {
        request_goal_transition(&worker, "test/early-goal").await;
    }
    let completing = Arc::clone(&worker);
    let t1 = tokio::spawn(async move {
        completing
            .finish(
                completing.server.generation(),
                i64::try_from(completing.server.generation()).unwrap(),
                &TurnCompletion {
                    thread_id: "thread".into(),
                    turn_id: "T1".into(),
                    status: TurnStatus::Completed,
                    error_message: String::new(),
                    interrupt_origin: None,
                    duration_ms: None,
                    usage_limit: false,
                },
            )
            .await
    });
    if !early {
        gate.entered.await.unwrap();
        assert!(
            !t1.is_finished(),
            "prior completion is genuinely blocked on HTTP"
        );
        assert!(queue::list(&db).unwrap()[0].goal_waiting);
        request_goal_transition(&worker, "test/advance-goal").await;
    }
    let started = notifications.recv().await.unwrap();
    let terminal = notifications.recv().await.unwrap();
    // Same observer-before-processor ordering as driver: processor is still in T1.
    worker.observe_terminal(&terminal).unwrap();
    assert!(mirror.poll_once().await.is_err());
    assert_eq!(get_offset(&db, "thread").unwrap().unwrap().cursor, 0);
    if early {
        assert!(!queue::list(&db).unwrap()[0].goal_waiting);
        assert!(cdr_store::observed_completion::contains(&db, "thread", "T1").unwrap());
        std::fs::write(rollout.with_extension("jsonl.release"), "release").unwrap();
    }
    gate.release.send(()).unwrap();
    t1.await.unwrap().unwrap();
    worker.handle(started).await.unwrap();
    assert_eq!(queue::list(&db).unwrap()[0].turn_id.as_deref(), Some("T2"));
    // Now completion owns T2; overlap actual final sender with mirror polling.
    let (final_result, mirror_result) = tokio::join!(worker.handle(terminal), mirror.poll_once());
    final_result.unwrap();
    if let Ok(poll) = mirror_result {
        assert_eq!(poll.sent, 0);
    }
    for _ in 0..2 {
        assert_eq!(mirror.poll_once().await.unwrap().sent, 0);
        worker.deliver_pending().await.unwrap();
        worker.recover_goal_progress().await.unwrap();
    }
    assert!(get_offset(&db, "thread").unwrap().unwrap().cursor > 0);
    assert!(cdr_store::delivery::list_pending(&db).unwrap().is_empty());
    assert!(cdr_store::goal_progress::pending(&db).unwrap().is_empty());
    worker.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(
        posts.len(),
        2,
        "exactly one progress and one final, no mirror POST"
    );
    assert_eq!(posts[0]["content"], "[Goal progress]\nprogress");
    assert!(posts[1]["content"].as_str().unwrap().starts_with("Final\n"));
    assert!(posts[1]["content"].as_str().unwrap().contains("goal final"));
    assert!(!posts[1]["content"].as_str().unwrap().contains("progress"));
    assert_ne!(posts[0]["nonce"], posts[1]["nonce"]);
}
