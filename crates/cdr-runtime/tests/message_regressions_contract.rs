use std::fmt::Write as _;
use std::fs;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use cdr_runtime::commentary_stream::CommentaryBuffer;
use cdr_runtime::session_mirror::{MirrorDetail, MirrorKind, collect_items};
use cdr_runtime::session_mirror_worker::{
    SessionMirrorDeliveryIdentity, SessionMirrorSender, SessionMirrorWorker,
};
use cdr_store::delivery::stage_queue_completion;
use cdr_store::mapping::upsert_thread;
use cdr_store::mirror::update_cursor;
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, mark_running};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

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
            self.0.lock().unwrap().push(text.to_owned());
            Ok(())
        })
    }
}

fn events(values: Vec<Value>) -> Vec<Map<String, Value>> {
    values
        .into_iter()
        .map(|v| v.as_object().unwrap().clone())
        .collect()
}

fn started(turn: &str) -> Value {
    json!({"type":"event_msg","payload":{"type":"task_started","turn_id":turn}})
}

fn user(text: &str) -> Value {
    json!({"timestamp":"2026-09-05T10:00:00Z","type":"event_msg",
        "payload":{"type":"user_message","message":text}})
}

#[test]
fn visible_agent_commentary_is_forwarded_without_reasoning_deltas() {
    let mut buffer = CommentaryBuffer::default();
    let block = buffer
        .observe(
            "item/completed",
            &json!({
                "threadId":"thread-1","turnId":"turn-1",
                "item":{"id":"message-1","type":"agentMessage","phase":"commentary",
                    "text":"수정하고 있습니다."}
            }),
        )
        .expect("the actual assistant commentary must reach Discord");
    assert_eq!(block.text, "수정하고 있습니다.");
    assert_eq!(block.turn_id, "turn-1");
}

#[test]
fn rollout_user_has_turn_identity_and_failed_completion_keeps_the_error() {
    let items = collect_items(
        "thread-1",
        &events(vec![
            started("turn-1"),
            user("hello"),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1",
            "error":{"message":"unsupported model"},"last_agent_message":null}}),
        ]),
        MirrorDetail::Send,
    );
    assert_eq!(items[0].turn_id.as_deref(), Some("turn-1"));
    assert_eq!(items[1].kind, MirrorKind::Failed);
    assert_eq!(items[1].text, "unsupported model");
}

async fn poll(values: Vec<Value>, completed: bool) -> Vec<String> {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let mirror = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(
        &rollout,
        values.iter().fold(String::new(), |mut text, value| {
            writeln!(text, "{value}").unwrap();
            text
        }),
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
    enqueue(
        &mirror,
        NewQueueJob {
            job_id: "job-1",
            target_thread_id: "thread-1",
            channel_id: 200,
            owner_user_id: Some(300),
            discord_message_id: Some(400),
            app_server_generation: 1,
            prompt: "hello",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_attempt(&mirror, "job-1", &[], 1).unwrap();
    mark_running(&mirror, "job-1", "discord-turn", 1).unwrap();
    if completed {
        stage_queue_completion(&mirror, "job-1", "ERROR: unsupported model", 2.0).unwrap();
    }
    update_cursor(
        &mirror,
        "thread-1",
        rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )
    .unwrap();
    let sender = Arc::new(Sender::default());
    SessionMirrorWorker::new(state, mirror, Arc::clone(&sender))
        .poll_once()
        .await
        .unwrap();
    sender.0.lock().unwrap().clone()
}

#[tokio::test]
async fn completed_discord_prompt_does_not_echo_but_a_new_app_repeat_is_visible() {
    let mut repeated = user("hello");
    repeated["timestamp"] = json!("2026-09-05T10:01:00Z");
    let messages = poll(
        vec![
            started("discord-turn"),
            user("hello"),
            started("app-turn"),
            repeated,
        ],
        true,
    )
    .await;
    assert_eq!(
        messages,
        vec!["Codex app user\n\nhello"],
        "only the new app input is mirrored, even with identical text"
    );
}

#[tokio::test]
async fn failed_discord_prompt_does_not_echo_after_queue_removal() {
    assert!(
        poll(vec![started("discord-turn"), user("hello")], true)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn stale_queue_for_another_turn_does_not_hide_app_commentary_or_final() {
    let messages = poll(vec![started("app-turn"),
        json!({"type":"event_msg","payload":{"type":"agent_message","phase":"commentary","message":"working"}}),
        json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"app-turn","last_agent_message":"done"}}),
    ], false).await;
    assert_eq!(messages, vec!["In progress\n\nworking", "Final\n\ndone"]);
}
