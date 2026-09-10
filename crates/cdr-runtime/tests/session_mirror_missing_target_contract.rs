use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cdr_runtime::session_mirror::SessionMirrorRetryState;
use cdr_runtime::session_mirror_worker::{
    SessionMirrorDeliveryIdentity, SessionMirrorError, SessionMirrorSender, SessionMirrorWorker,
};
use cdr_store::mapping::upsert_thread;
use cdr_store::mirror::{get_offset, update_cursor};
use rusqlite::{Connection, params};

#[derive(Default)]
struct RecordingSender {
    messages: Mutex<Vec<(u64, String)>>,
}

impl SessionMirrorSender for RecordingSender {
    fn send<'a>(
        &'a self,
        channel_id: u64,
        _identity: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            self.messages
                .lock()
                .unwrap()
                .push((channel_id, text.to_owned()));
            Ok(())
        })
    }
}

#[tokio::test]
async fn missing_target_is_throttled_without_starving_a_later_target() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let live_rollout = temp.path().join("live.jsonl");
    let unavailable_rollout = temp.path().join("unavailable.jsonl");
    fs::write(
        &live_rollout,
        "{\"timestamp\":\"1\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"later target progressed\"}}\n",
    )
    .unwrap();
    seed_state(&state_db, &live_rollout);
    upsert_thread(
        &mirror_db,
        "thread-missing",
        "project",
        "Missing",
        100,
        201,
        2.0,
    )
    .unwrap();
    upsert_thread(&mirror_db, "thread-live", "project", "Live", 100, 202, 1.0).unwrap();
    update_cursor(
        &mirror_db,
        "thread-live",
        live_rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    let sender = Arc::new(RecordingSender::default());
    let worker = SessionMirrorWorker::new(state_db.clone(), mirror_db.clone(), Arc::clone(&sender));
    let mut retry = SessionMirrorRetryState::default();

    let first_error = worker
        .poll_once()
        .await
        .expect_err("a missing mapped target must be an aggregate poll failure");
    assert_missing_target_error(&first_error, 1, 1, "mapped Codex thread is unavailable");
    assert_eq!(
        sender.messages.lock().unwrap().as_slice(),
        &[(202, "In progress\n\nlater target progressed".to_owned())]
    );
    assert_eq!(
        get_offset(&mirror_db, "thread-live")
            .unwrap()
            .unwrap()
            .cursor,
        i64::try_from(fs::metadata(&live_rollout).unwrap().len()).unwrap()
    );
    let first_text = first_error.to_string();
    let first = retry.on_failure(Duration::ZERO, &first_text);
    assert_eq!(first.retry_after, Duration::from_secs(1));
    assert_eq!(first.report.unwrap().error, first_text);

    let second_error = worker.poll_once().await.unwrap_err();
    assert_missing_target_error(&second_error, 0, 0, "mapped Codex thread is unavailable");
    assert_eq!(
        second_error.to_string(),
        first_text,
        "partial progress must not turn one persistent failure into a new log identity"
    );
    let second = retry.on_failure(Duration::from_secs(1), &second_error.to_string());
    assert_eq!(second.retry_after, Duration::from_secs(2));
    assert!(second.report.is_none());

    for (now, expected_delay, should_report) in [
        (3, 4, false),
        (7, 8, false),
        (15, 16, false),
        (31, 30, false),
        (61, 30, true),
    ] {
        let error = worker.poll_once().await.unwrap_err();
        let exact_error = error.to_string();
        let decision = retry.on_failure(Duration::from_secs(now), &exact_error);
        assert_eq!(decision.retry_after, Duration::from_secs(expected_delay));
        assert_eq!(decision.report.is_some(), should_report);
        if let Some(report) = decision.report {
            assert_eq!(report.count, 7);
            assert_eq!(report.error, exact_error);
        }
    }
    assert_eq!(sender.messages.lock().unwrap().len(), 1);

    insert_thread(&state_db, "thread-missing", &unavailable_rollout, 2);
    let missing_rollout = worker.poll_once().await.unwrap_err();
    assert_missing_target_error(
        &missing_rollout,
        0,
        0,
        "mapped Codex thread rollout file is unavailable",
    );
    fs::write(&unavailable_rollout, "").unwrap();
    worker.poll_once().await.unwrap();
    assert_eq!(retry.on_success(), Duration::from_secs(1));
    Connection::open(&state_db)
        .unwrap()
        .execute("DELETE FROM threads WHERE id='thread-missing'", [])
        .unwrap();
    let after_recovery = worker.poll_once().await.unwrap_err();
    let reset = retry.on_failure(Duration::from_secs(62), &after_recovery.to_string());
    assert_eq!(reset.retry_after, Duration::from_secs(1));
    assert_eq!(reset.report.unwrap().count, 1);
}

fn assert_missing_target_error(
    error: &SessionMirrorError,
    events: usize,
    sent: usize,
    expected_error: &str,
) {
    let SessionMirrorError::TargetBatch {
        failed_targets,
        progress,
        first_target,
        first_error,
        ..
    } = error
    else {
        panic!("expected target batch, got {error}");
    };
    assert_eq!(*failed_targets, 1);
    assert_eq!(first_target, "thread-missing");
    assert_eq!(progress.targets, 2);
    assert_eq!(progress.events, events);
    assert_eq!(progress.sent, sent);
    assert_eq!(first_error.to_string(), expected_error);
}

fn seed_state(path: &Path, live_rollout: &Path) {
    let connection = Connection::open(path).unwrap();
    connection.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT);").unwrap();
    drop(connection);
    insert_thread(path, "thread-live", live_rollout, 1);
}

fn insert_thread(path: &Path, thread_id: &str, rollout: &Path, updated_at: i64) {
    Connection::open(path)
        .unwrap()
        .execute(
            "INSERT INTO threads VALUES (?1,?1,'C:/repo',?2,?3,'gpt','high',0,0,0,'vscode','user')",
            params![thread_id, updated_at, rollout.to_string_lossy().as_ref()],
        )
        .unwrap();
}
