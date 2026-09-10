use std::{
    fs,
    future::Future,
    io::Write,
    pin::Pin,
    sync::{Arc, Mutex},
};

use cdr_runtime::session_mirror_worker::{
    SessionMirrorDeliveryIdentity, SessionMirrorSender, SessionMirrorWorker,
};
use cdr_store::{mapping::upsert_thread, mirror::update_cursor};
use rusqlite::Connection;
use serde_json::json;

#[derive(Default)]
struct Sender(Mutex<Vec<String>>);

impl SessionMirrorSender for Sender {
    fn send<'a>(
        &'a self,
        _: u64,
        _: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(text.into());
            Ok(())
        })
    }
}

fn append_turn(path: &std::path::Path, turn: &str, timestamp: Option<&str>) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"event_msg","payload":{
            "type":"task_started","turn_id":turn
        }})
    )
    .unwrap();
    for kind in ["user_message", "agent_message"] {
        let mut event = json!({"type":"event_msg","payload":{
            "type":kind,"message":"same text"
        }});
        if let Some(timestamp) = timestamp {
            event["timestamp"] = timestamp.into();
        }
        writeln!(file, "{event}").unwrap();
    }
}

async fn verify(timestamp: Option<&str>, split: bool) {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let mirror = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(&rollout, "").unwrap();
    let db = Connection::open(&state).unwrap();
    db.execute_batch("CREATE TABLE threads (id TEXT, title TEXT, cwd TEXT, updated_at INTEGER,
        rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER);").unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('thread-1','title','C:/repo',1,?,'gpt','high',0,0)",
        [rollout.to_string_lossy().as_ref()],
    )
    .unwrap();
    upsert_thread(&mirror, "thread-1", "project", "title", 100, 200, 1.0).unwrap();
    update_cursor(
        &mirror,
        "thread-1",
        rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    let sender = Arc::new(Sender::default());
    append_turn(&rollout, "turn-1", timestamp);
    if split {
        let worker = SessionMirrorWorker::new(state.clone(), mirror.clone(), sender.clone());
        assert_eq!(worker.poll_once().await.unwrap().sent, 2);
        drop(worker);
    }
    append_turn(&rollout, "turn-2", timestamp);
    let worker = SessionMirrorWorker::new(state.clone(), mirror.clone(), sender.clone());
    assert_eq!(
        worker.poll_once().await.unwrap().sent,
        if split { 2 } else { 4 },
        "equal or absent timestamps must not merge distinct turns"
    );
    assert_eq!(
        sender.0.lock().unwrap().as_slice(),
        &[
            "Codex app user\n\nsame text",
            "In progress\n\nsame text",
            "Codex app user\n\nsame text",
            "In progress\n\nsame text",
        ]
    );
    // Replayed records in the same turn remain suppressed after reconstruction.
    append_turn(&rollout, "turn-2", timestamp);
    drop(worker);
    let worker = SessionMirrorWorker::new(state, mirror, sender.clone());
    assert_eq!(worker.poll_once().await.unwrap().sent, 0);
    assert_eq!(sender.0.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn equal_timestamps_preserve_distinct_turns_in_one_poll() {
    verify(Some("same"), false).await;
}

#[tokio::test]
async fn absent_timestamps_preserve_distinct_turns_in_one_poll() {
    verify(None, false).await;
}

#[tokio::test]
async fn equal_timestamps_preserve_distinct_turns_after_restart() {
    verify(Some("same"), true).await;
}

#[tokio::test]
async fn absent_timestamps_preserve_distinct_turns_after_restart() {
    verify(None, true).await;
}
