use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cdr_runtime::session_mirror::{MirrorDetail, MirrorKind, collect_items};
use cdr_runtime::session_mirror_worker::{
    SESSION_MIRROR_ASSISTANT_TEXT_NONCE_DOMAIN, SessionMirrorDeliveryIdentity, SessionMirrorSender,
    SessionMirrorWorker,
};
use cdr_store::mapping::upsert_thread;
use cdr_store::mirror::{get_offset, update_cursor};
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, mark_running};
use rusqlite::Connection;
use serde_json::json;

fn event(mut value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    std::mem::take(value.as_object_mut().unwrap())
}

#[test]
fn rollout_events_become_user_commentary_final_and_optional_activity() {
    let events = vec![
        event(
            json!({"timestamp":"1","type":"event_msg","payload":{"type":"user_message","message":"from app"}}),
        ),
        event(
            json!({"timestamp":"2","type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"working"}]}}),
        ),
        event(
            json!({"timestamp":"3","type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"checking"}]}}),
        ),
        event(
            json!({"timestamp":"4","type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"duplicate final"}]}}),
        ),
        event(
            json!({"timestamp":"5","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1","last_agent_message":"done"}}),
        ),
        event(
            json!({"timestamp":"6","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions\ninternal"}]}}),
        ),
    ];

    let send = collect_items("thread-1", &events, MirrorDetail::Send);
    assert_eq!(
        send.iter()
            .map(|item| (item.kind, item.text.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (MirrorKind::User, "from app"),
            (MirrorKind::Commentary, "working"),
            (MirrorKind::Final, "done"),
        ]
    );
    assert_eq!(
        send[1].digest, "771270a7b689873878753c03a9a8c01b8d8a0b94b11060d99479e3a27ca44c4f",
        "Rust must use the Python v2 session-mirror digest contract",
    );
    let all = collect_items("thread-1", &events, MirrorDetail::All);
    assert!(all.iter().any(|item| item.text == "checking"));
    assert!(!all.iter().any(|item| item.text.contains("duplicate final")));
    assert!(!all.iter().any(|item| item.text.contains("AGENTS.md")));
    assert!(all.iter().all(|item| item.digest.len() == 64));
}

#[derive(Default)]
struct FakeSender {
    fail_once: AtomicBool,
    attempts: Mutex<Vec<(u64, String, String, String)>>,
    messages: Mutex<Vec<(u64, String, String, String)>>,
}

impl SessionMirrorSender for FakeSender {
    fn send<'a>(
        &'a self,
        channel_id: u64,
        identity: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let attempted = (
                channel_id,
                identity.domain().into(),
                identity.logical_key().into(),
                text.into(),
            );
            self.attempts.lock().unwrap().push(attempted.clone());
            if self.fail_once.swap(false, Ordering::SeqCst) {
                return Err("injected Discord failure".into());
            }
            self.messages.lock().unwrap().push(attempted);
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
async fn cursor_and_digest_commit_only_after_discord_delivery_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(
        &rollout,
        "{\"timestamp\":\"1\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"phase\":\"commentary\",\"message\":\"hello\"}}\n",
    ).unwrap();
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
    sender.fail_once.store(true, Ordering::SeqCst);
    let worker = SessionMirrorWorker::new(state_db, mirror_db.clone(), Arc::clone(&sender));

    let first = worker.poll_once().await.unwrap_err();
    assert!(first.to_string().contains("injected Discord failure"));
    assert_eq!(
        get_offset(&mirror_db, "thread-1").unwrap().unwrap().cursor,
        0
    );
    assert!(sender.messages.lock().unwrap().is_empty());
    assert_eq!(sender.attempts.lock().unwrap().len(), 1);

    let result = worker.poll_once().await.unwrap();
    assert_eq!(result.sent, 1);
    {
        let attempts = sender.attempts.lock().unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], attempts[1]);
    }
    assert_eq!(
        sender.messages.lock().unwrap().as_slice(),
        &[(
            200,
            SESSION_MIRROR_ASSISTANT_TEXT_NONCE_DOMAIN.into(),
            "13:8:thread-1:0::f3aefe62965a91903610f0e23cc8a69d5b87cea6d28e75489b0d2ca02ed7993c"
                .into(),
            "In progress\n\nhello".into(),
        )]
    );
    assert_eq!(
        get_offset(&mirror_db, "thread-1").unwrap().unwrap().cursor,
        i64::try_from(fs::metadata(&rollout).unwrap().len()).unwrap()
    );
    let repeated = worker.poll_once().await.unwrap();
    assert_eq!(repeated.sent, 0);
    assert_eq!(sender.messages.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn discord_origin_final_is_not_mirrored_after_outbox_delivery_is_removed() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(
        &rollout,
        "{\"timestamp\":\"1\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"turn-1\",\"last_agent_message\":\"done\"}}\n",
    )
    .unwrap();
    seed_state(&state_db, &rollout);
    upsert_thread(&mirror_db, "thread-1", "project", "Title", 100, 200, 1.0).unwrap();
    let queued = enqueue(
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
    assert!(queued.created);
    begin_attempt(&mirror_db, "job-1", &[], 1).unwrap();
    mark_running(&mirror_db, "job-1", "turn-1", 1).unwrap();
    let delivery =
        cdr_store::delivery::stage_queue_completion(&mirror_db, "job-1", "done", 2.0).unwrap();
    assert!(cdr_store::delivery::complete(&mirror_db, &delivery.delivery_id).unwrap());
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
async fn first_mapping_primes_to_eof_instead_of_replaying_old_history() {
    let temp = tempfile::tempdir().unwrap();
    let state_db = temp.path().join("state.sqlite");
    let mirror_db = temp.path().join("mirror.sqlite");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(
        &rollout,
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"old\"}}\n",
    )
    .unwrap();
    seed_state(&state_db, &rollout);
    upsert_thread(&mirror_db, "thread-1", "project", "Title", 100, 200, 1.0).unwrap();
    let sender = Arc::new(FakeSender::default());
    let worker = SessionMirrorWorker::new(state_db, mirror_db.clone(), Arc::clone(&sender));

    assert_eq!(worker.poll_once().await.unwrap().sent, 0);
    assert_eq!(
        get_offset(&mirror_db, "thread-1").unwrap().unwrap().cursor,
        i64::try_from(fs::metadata(&rollout).unwrap().len()).unwrap()
    );
    assert!(sender.messages.lock().unwrap().is_empty());
}
