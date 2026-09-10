use cdr_runtime::session_mirror_worker::{
    SessionMirrorDeliveryIdentity, SessionMirrorSender, SessionMirrorWorker,
};
use cdr_store::{
    mapping::upsert_thread,
    mirror::{get_cursor_turn, record_user_origin, update_cursor},
};
use rusqlite::Connection;
use std::{
    fs,
    future::Future,
    io::Write,
    pin::Pin,
    sync::{Arc, Mutex},
};

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

#[tokio::test]
async fn turn_identity_survives_split_polls_and_worker_restart() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let mirror = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(
        &rollout,
        "{\"type\":\"turn_context\",\"payload\":{\"turn_id\":\"discord-turn\"}}\n",
    )
    .unwrap();
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
    record_user_origin(&mirror, "thread-1", "discord-turn", "hello", 1.0).unwrap();
    let sender = Arc::new(Sender::default());
    let worker = SessionMirrorWorker::new(state.clone(), mirror.clone(), sender.clone());
    assert_eq!(worker.poll_once().await.unwrap().sent, 0);
    assert_eq!(
        get_cursor_turn(&mirror, "thread-1").unwrap().as_deref(),
        Some("discord-turn")
    );
    fs::OpenOptions::new().append(true).open(&rollout).unwrap().write_all(
        b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hello\"}}\n").unwrap();
    drop(worker);
    let worker = SessionMirrorWorker::new(state, mirror, sender.clone());
    assert_eq!(worker.poll_once().await.unwrap().sent, 0);
    assert!(sender.0.lock().unwrap().is_empty());
}
