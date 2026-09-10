use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use cdr_runtime::session_mirror_worker::{
    SessionMirrorDeliveryIdentity, SessionMirrorError, SessionMirrorSender, SessionMirrorWorker,
};
use cdr_store::mapping::upsert_thread;
use cdr_store::mirror::{get_offset, update_cursor};
use rusqlite::{Connection, params};

struct FailingSender {
    fail_channel: u64,
    attempts: Mutex<Vec<u64>>,
    messages: Mutex<Vec<(u64, String)>>,
}

impl SessionMirrorSender for FailingSender {
    fn send<'a>(
        &'a self,
        channel_id: u64,
        _identity: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            self.attempts.lock().unwrap().push(channel_id);
            if channel_id == self.fail_channel {
                return Err("persistent target failure".into());
            }
            self.messages
                .lock()
                .unwrap()
                .push((channel_id, text.into()));
            Ok(())
        })
    }
}

fn seed_state(path: &Path, first_rollout: &Path, second_rollout: &Path) {
    let connection = Connection::open(path).unwrap();
    connection.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT);").unwrap();
    for (thread, title, updated, rollout) in [
        ("thread-a", "A", 2_i64, first_rollout),
        ("thread-b", "B", 1_i64, second_rollout),
    ] {
        connection
            .execute(
                "INSERT INTO threads VALUES (?1,?2,'C:/repo',?3,?4,'gpt','high',0,0,0,'vscode','user')",
                params![thread, title, updated, rollout.to_string_lossy().as_ref()],
            )
            .unwrap();
    }
}

#[tokio::test]
async fn failing_first_target_does_not_starve_later_targets() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let first_rollout = temp.path().join("a.jsonl");
    let second_rollout = temp.path().join("b.jsonl");
    fs::write(
        &first_rollout,
        "{\"timestamp\":\"1\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"first\"}}\n",
    )
    .unwrap();
    fs::write(
        &second_rollout,
        "{\"timestamp\":\"2\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"second\"}}\n",
    )
    .unwrap();
    seed_state(&state_db, &first_rollout, &second_rollout);
    upsert_thread(&mirror_db, "thread-a", "project", "A", 100, 201, 2.0).unwrap();
    upsert_thread(&mirror_db, "thread-b", "project", "B", 100, 202, 1.0).unwrap();
    update_cursor(
        &mirror_db,
        "thread-a",
        first_rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    update_cursor(
        &mirror_db,
        "thread-b",
        second_rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    let sender = Arc::new(FailingSender {
        fail_channel: 201,
        attempts: Mutex::new(Vec::new()),
        messages: Mutex::new(Vec::new()),
    });
    let worker = SessionMirrorWorker::new(state_db, mirror_db.clone(), Arc::clone(&sender));

    let error = worker.poll_once().await.unwrap_err();

    let SessionMirrorError::TargetBatch {
        failed_targets,
        progress,
        first_target,
        ..
    } = &error
    else {
        panic!("expected target batch error, got {error}");
    };
    assert_eq!(*failed_targets, 1);
    assert_eq!(*first_target, "thread-a");
    assert_eq!(progress.targets, 2);
    assert_eq!(progress.events, 1);
    assert_eq!(progress.sent, 1);
    assert!(error.to_string().contains("thread-a"));
    assert!(error.to_string().contains("persistent target failure"));
    assert_eq!(sender.attempts.lock().unwrap().as_slice(), &[201, 202]);
    assert_eq!(
        sender.messages.lock().unwrap().as_slice(),
        &[(202, "In progress\n\nsecond".into())]
    );
    assert_eq!(
        get_offset(&mirror_db, "thread-a").unwrap().unwrap().cursor,
        0
    );
    assert_eq!(
        get_offset(&mirror_db, "thread-b").unwrap().unwrap().cursor,
        i64::try_from(fs::metadata(&second_rollout).unwrap().len()).unwrap()
    );
}

#[tokio::test]
async fn missing_first_target_does_not_hide_a_later_delivery_failure() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let first_rollout = temp.path().join("a.jsonl");
    let second_rollout = temp.path().join("b.jsonl");
    fs::write(&first_rollout, "").unwrap();
    fs::write(
        &second_rollout,
        "{\"timestamp\":\"2\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"second\"}}\n",
    )
    .unwrap();
    seed_state(&state_db, &first_rollout, &second_rollout);
    Connection::open(&state_db)
        .unwrap()
        .execute("DELETE FROM threads WHERE id='thread-a'", [])
        .unwrap();
    upsert_thread(&mirror_db, "thread-a", "project", "A", 100, 201, 2.0).unwrap();
    upsert_thread(&mirror_db, "thread-b", "project", "B", 100, 202, 1.0).unwrap();
    update_cursor(
        &mirror_db,
        "thread-b",
        second_rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    let sender = Arc::new(FailingSender {
        fail_channel: 202,
        attempts: Mutex::new(Vec::new()),
        messages: Mutex::new(Vec::new()),
    });
    let worker = SessionMirrorWorker::new(state_db, mirror_db.clone(), Arc::clone(&sender));

    let error = worker.poll_once().await.unwrap_err();

    let SessionMirrorError::TargetBatch {
        failed_targets,
        progress,
        first_target,
        ..
    } = &error
    else {
        panic!("expected target batch, got {error}");
    };
    assert_eq!(*failed_targets, 2);
    assert_eq!(progress.targets, 2);
    assert_eq!(progress.events, 0);
    assert_eq!(progress.sent, 0);
    assert_eq!(first_target, "thread-a");
    assert!(error.is_delivery_failure());
    assert!(error.to_string().contains("thread-a"));
    assert!(error.to_string().contains("thread-b"));
    assert!(error.to_string().contains("persistent target failure"));
    assert_eq!(sender.attempts.lock().unwrap().as_slice(), &[202]);

    let first_text = error.to_string();
    upsert_thread(&mirror_db, "thread-a", "project", "A", 100, 201, 1.0).unwrap();
    upsert_thread(&mirror_db, "thread-b", "project", "B", 100, 202, 2.0).unwrap();
    let reordered = worker.poll_once().await.unwrap_err();
    assert_eq!(
        reordered.to_string(),
        first_text,
        "mapping recency must not redefine one persistent failure set"
    );
    assert_eq!(sender.attempts.lock().unwrap().as_slice(), &[202, 202]);
}
