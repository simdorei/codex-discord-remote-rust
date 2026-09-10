use std::fs;
use std::future::Future;
use std::io::Write;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use cdr_runtime::session_mirror::{MirrorDetail, MirrorKind, collect_items};
use cdr_runtime::session_mirror_worker::{
    SessionMirrorDeliveryIdentity, SessionMirrorSender, SessionMirrorWorker,
};
use cdr_store::mapping::upsert_thread;
use cdr_store::mirror::update_cursor;
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, mark_running};
use rusqlite::Connection;
use serde_json::json;

fn event(mut value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    std::mem::take(value.as_object_mut().unwrap())
}

#[test]
fn missing_agent_message_phase_defaults_to_python_commentary_digest() {
    let events = vec![event(json!({
        "timestamp": "1",
        "type": "event_msg",
        "payload": {"type": "agent_message", "message": "working"}
    }))];

    let items = collect_items("thread-1", &events, MirrorDetail::Send);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, MirrorKind::Commentary);
    assert_eq!(items[0].phase, "commentary");
    assert_eq!(
        items[0].digest,
        "36357d4878fa5dbf0c7fb367a21205f3d801c53cb0a8e2319c53bb358b69aa01"
    );
}

#[test]
fn all_mode_activity_digests_include_python_item_index_rehash() {
    let events = vec![event(json!({
        "timestamp": "2",
        "type": "response_item",
        "payload": {
            "type": "reasoning",
            "summary": [
                {"type": "summary_text", "text": "checking"},
                {"type": "summary_text", "text": "checking"}
            ]
        }
    }))];

    let items = collect_items("thread-1", &events, MirrorDetail::All);

    assert_eq!(items.len(), 2, "Python preserves duplicate activity parts");
    assert_eq!(
        items
            .iter()
            .map(|item| item.digest.as_str())
            .collect::<Vec<_>>(),
        vec![
            "c8dea18ba5692261e9d7044748b921d649f5ac8e6824dc317b0375f8cbaa8673",
            "bc830572b59debef8929edca66645494ff13e7b59225c4e241b2ed6c2951064c",
        ]
    );
}

#[derive(Default)]
struct FakeSender {
    messages: Mutex<Vec<(u64, String)>>,
}

impl SessionMirrorSender for FakeSender {
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
                .push((channel_id, text.into()));
            Ok(())
        })
    }
}

fn seed_state(path: &Path, rollout: &Path) {
    let connection = Connection::open(path).unwrap();
    connection.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT);").unwrap();
    connection.execute(
        "INSERT INTO threads VALUES ('thread-1','Title','C:/repo',1,?1,'gpt','high',0,0,0,'vscode','user')",
        [rollout.to_string_lossy().as_ref()],
    ).unwrap();
}

#[tokio::test]
async fn discord_origin_commentary_is_not_duplicated_by_the_session_mirror() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(
        &rollout,
        concat!(
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-1\"}}\n",
            "{\"timestamp\":\"1\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"phase\":\"commentary\",\"message\":\"working\"}}\n"
        ),
    )
    .unwrap();
    seed_state(&state_db, &rollout);
    upsert_thread(&mirror_db, "thread-1", "project", "Title", 100, 200, 1.0).unwrap();
    enqueue(
        &mirror_db,
        NewQueueJob {
            job_id: "job-1",
            target_thread_id: "thread-1",
            channel_id: 200,
            owner_user_id: Some(300),
            discord_message_id: Some(400),
            app_server_generation: 1,
            prompt: "prompt",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_attempt(&mirror_db, "job-1", &[], 1).unwrap();
    mark_running(&mirror_db, "job-1", "turn-1", 1).unwrap();
    update_cursor(
        &mirror_db,
        "thread-1",
        rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    let sender = Arc::new(FakeSender::default());
    let worker = SessionMirrorWorker::new(state_db, mirror_db, Arc::clone(&sender));

    let result = worker.poll_once().await.unwrap();

    assert_eq!(result.sent, 0);
    assert!(sender.messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn recent_assistant_text_is_deduped_across_event_shapes_and_polls() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(
        &rollout,
        "{\"timestamp\":\"1\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"  same update  \"}}\n",
    )
    .unwrap();
    seed_state(&state_db, &rollout);
    upsert_thread(&mirror_db, "thread-1", "project", "Title", 100, 200, 1.0).unwrap();
    update_cursor(
        &mirror_db,
        "thread-1",
        rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    let sender = Arc::new(FakeSender::default());
    let worker = SessionMirrorWorker::new(state_db, mirror_db, Arc::clone(&sender));

    assert_eq!(worker.poll_once().await.unwrap().sent, 1);
    fs::OpenOptions::new()
        .append(true)
        .open(&rollout)
        .unwrap()
        .write_all(
            concat!(
                "{\"timestamp\":\"2\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"commentary\",\"content\":[{\"type\":\"output_text\",\"text\":\"same update\"}]}}\n",
                "{\"timestamp\":\"3\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"turn-1\",\"last_agent_message\":\"same update\"}}\n"
            )
            .as_bytes(),
        )
        .unwrap();

    assert_eq!(
        worker.poll_once().await.unwrap().sent,
        1,
        "Final is a distinct user-visible outcome, even with identical text"
    );
    assert_eq!(
        sender.messages.lock().unwrap().as_slice(),
        &[
            (200, "In progress\n\nsame update".into()),
            (200, "Final\n\nsame update".into())
        ]
    );
    assert_eq!(worker.poll_once().await.unwrap().sent, 0);
    fs::OpenOptions::new().append(true).open(&rollout).unwrap().write_all(concat!(
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-2\"}}\n",
        "{\"timestamp\":\"4\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"same update\"}}\n"
    ).as_bytes()).unwrap();
    assert_eq!(
        worker.poll_once().await.unwrap().sent,
        1,
        "a new turn must not inherit the previous turn's text suppression"
    );
}
