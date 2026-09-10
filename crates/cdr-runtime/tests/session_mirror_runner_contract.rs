use std::fs;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use cdr_runtime::session_mirror::{
    SessionMirrorDeliveryIdentity, SessionMirrorRetryState, SessionMirrorSender,
    run_session_mirror_worker,
};
use cdr_runtime::session_mirror_worker::SessionMirrorWorker;
use cdr_store::mapping::upsert_thread;
use cdr_store::mirror::update_cursor;
use rusqlite::{Connection, params};
use tokio::sync::Notify;
use tokio::time::timeout;

struct BlockingSender {
    started: Notify,
}

impl SessionMirrorSender for BlockingSender {
    fn send<'a>(
        &'a self,
        _channel_id: u64,
        _identity: &'a SessionMirrorDeliveryIdentity,
        _text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            self.started.notify_one();
            std::future::pending::<Result<(), String>>().await
        })
    }
}

#[test]
fn identical_failures_back_off_and_report_on_a_bounded_cadence() {
    let mut state = SessionMirrorRetryState::default();
    let error = "Discord delivery failed: HTTP 502";

    let first = state.on_failure(Duration::ZERO, error);
    assert_eq!(first.retry_after, Duration::from_secs(1));
    assert_eq!(
        first.report.expect("first failure must be reported").count,
        1
    );

    for (now, expected_delay) in [(1, 2), (3, 4), (7, 8), (15, 16), (31, 30)] {
        let decision = state.on_failure(Duration::from_secs(now), error);
        assert_eq!(decision.retry_after, Duration::from_secs(expected_delay));
        assert!(decision.report.is_none());
    }

    let cadence = state.on_failure(Duration::from_secs(61), error);
    assert_eq!(cadence.retry_after, Duration::from_secs(30));
    let report = cadence
        .report
        .expect("the exact persistent error must be reported again within 60 seconds");
    assert_eq!(report.count, 7);
    assert_eq!(report.error, error);

    let quiet = state.on_failure(Duration::from_secs(91), error);
    assert!(quiet.report.is_none());
    let next_cadence = state.on_failure(Duration::from_secs(121), error);
    assert_eq!(
        next_cadence.report.expect("bounded report cadence").count,
        9
    );
}

#[test]
fn changed_error_is_reported_immediately_with_a_fresh_backoff() {
    let mut state = SessionMirrorRetryState::default();
    let _ = state.on_failure(Duration::ZERO, "HTTP 502");
    let _ = state.on_failure(Duration::from_secs(1), "HTTP 502");

    let changed = state.on_failure(Duration::from_secs(3), "database locked");

    assert_eq!(changed.retry_after, Duration::from_secs(1));
    let report = changed
        .report
        .expect("a changed exact error must be visible");
    assert_eq!(report.count, 1);
    assert_eq!(report.error, "database locked");
}

#[test]
fn success_resets_failure_count_backoff_and_reporting() {
    let mut state = SessionMirrorRetryState::default();
    let _ = state.on_failure(Duration::ZERO, "HTTP 502");
    let _ = state.on_failure(Duration::from_secs(1), "HTTP 502");

    assert_eq!(state.on_success(), Duration::from_secs(1));

    let after_recovery = state.on_failure(Duration::from_secs(2), "HTTP 502");
    assert_eq!(after_recovery.retry_after, Duration::from_secs(1));
    let report = after_recovery
        .report
        .expect("the first failure after recovery must be reported");
    assert_eq!(report.count, 1);
    assert_eq!(report.error, "HTTP 502");
}

#[tokio::test]
async fn pre_signalled_shutdown_exits_without_polling() {
    let temp = tempfile::tempdir().unwrap();
    let sender = Arc::new(BlockingSender {
        started: Notify::new(),
    });
    let worker = SessionMirrorWorker::new(
        temp.path().join("missing-state.sqlite"),
        temp.path().join("missing-mirror.sqlite"),
        sender,
    );
    let (_shutdown, receiver) = tokio::sync::watch::channel(true);

    timeout(
        Duration::from_millis(250),
        run_session_mirror_worker(worker, receiver),
    )
    .await
    .expect("pre-signalled shutdown must not wait for a later watch change");
}

#[tokio::test]
async fn shutdown_interrupts_an_in_flight_poll() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(
        &rollout,
        "{\"timestamp\":\"1\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"hello\"}}\n",
    )
    .unwrap();
    seed_state(&state_db, &rollout);
    upsert_thread(&mirror_db, "thread-a", "project", "A", 100, 201, 1.0).unwrap();
    update_cursor(
        &mirror_db,
        "thread-a",
        rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    let sender = Arc::new(BlockingSender {
        started: Notify::new(),
    });
    let worker = SessionMirrorWorker::new(state_db, mirror_db, Arc::clone(&sender));
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(run_session_mirror_worker(worker, receiver));
    timeout(Duration::from_secs(2), sender.started.notified())
        .await
        .expect("the controlled send must start");

    shutdown.send(true).unwrap();

    timeout(Duration::from_millis(250), task)
        .await
        .expect("shutdown must cancel a stalled poll")
        .unwrap();
}

fn seed_state(path: &std::path::Path, rollout: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT);").unwrap();
    connection
        .execute(
            "INSERT INTO threads VALUES ('thread-a','A','C:/repo',1,?1,'gpt','high',0,0,0,'vscode','user')",
            params![rollout.to_string_lossy().as_ref()],
        )
        .unwrap();
}
