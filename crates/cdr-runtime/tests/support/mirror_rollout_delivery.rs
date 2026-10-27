use super::rollout_recovery::{archive, history, save_cursor};
use super::*;
use cdr_runtime::session_mirror_worker::{
    SessionMirrorDeliveryIdentity, SessionMirrorSender, SessionMirrorWorker,
};
use cdr_store::mirror::get_offset;
use std::{future::Future, io::Write, pin::Pin};

#[derive(Default)]
struct Sender(Mutex<Vec<(u64, String)>>);

impl SessionMirrorSender for Sender {
    fn send<'a>(
        &'a self,
        channel: u64,
        _: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            self.0.lock().unwrap().push((channel, text.into()));
            Ok(())
        })
    }
}

#[tokio::test]
async fn manual_sync_then_new_answer_uses_existing_room_once_without_replaying_history() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, _) = fixture(&temp);
    let bytes = history("thread-a");
    save_cursor(&temp, i64::try_from(bytes.len()).unwrap());
    archive(&temp, &bytes);
    assert_eq!(sync.sync(99, Some(1)).await.unwrap().threads, 1);
    let db = temp.path().join("mirror.sqlite");
    Connection::open(&db)
        .unwrap()
        .execute(
            "DELETE FROM mirror_threads WHERE codex_thread_id<>'thread-a'",
            [],
        )
        .unwrap();
    let sender = Arc::new(Sender::default());
    let worker =
        SessionMirrorWorker::new(temp.path().join("state.sqlite"), db.clone(), sender.clone());
    assert_eq!(worker.poll_once().await.unwrap().sent, 0);
    let plain = temp.path().join("rollout.jsonl");
    let mut writer = fs::OpenOptions::new().append(true).open(&plain).unwrap();
    writeln!(writer, "{}", serde_json::json!({
        "timestamp":"new", "type":"event_msg",
        "payload":{"type":"task_complete","turn_id":"new-turn","last_agent_message":"new answer after sync"}
    })).unwrap();
    writer.sync_all().unwrap();
    assert_eq!(worker.poll_once().await.unwrap().sent, 1);
    assert_eq!(worker.poll_once().await.unwrap().sent, 0);
    let messages = sender.0.lock().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].0, 30);
    assert!(messages[0].1.contains("new answer after sync"));
    assert!(!messages[0].1.contains("기존 답변"));
    assert_eq!(
        u64::try_from(get_offset(&db, "thread-a").unwrap().unwrap().cursor).unwrap(),
        fs::metadata(plain).unwrap().len()
    );
}

#[tokio::test]
async fn recovered_mid_record_cursor_reuses_delivery_dedup_without_resending_history() {
    for compressed in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let (sync, _) = fixture(&temp);
        let bytes = history("thread-a");
        let plain = temp.path().join("rollout.jsonl");
        fs::write(&plain, &bytes).unwrap();
        let record_start = bytes.iter().position(|byte| *byte == b'\n').unwrap() + 1;
        save_cursor(&temp, i64::try_from(record_start).unwrap());
        let db = temp.path().join("mirror.sqlite");
        Connection::open(&db)
            .unwrap()
            .execute(
                "DELETE FROM mirror_threads WHERE codex_thread_id<>'thread-a'",
                [],
            )
            .unwrap();
        let sender = Arc::new(Sender::default());
        let worker = SessionMirrorWorker::new(
            temp.path().join("state.sqlite"),
            db.clone(),
            Arc::clone(&sender),
        );
        assert_eq!(worker.poll_once().await.unwrap().sent, 1);
        assert_eq!(sender.0.lock().unwrap().len(), 1);

        let broken_cursor = record_start + 17;
        save_cursor(&temp, i64::try_from(broken_cursor).unwrap());
        if compressed {
            archive(&temp, &bytes);
        }
        sync.sync(99, Some(1)).await.unwrap();
        assert_eq!(
            get_offset(&db, "thread-a").unwrap().unwrap().cursor,
            i64::try_from(record_start).unwrap()
        );

        assert_eq!(worker.poll_once().await.unwrap().sent, 0);
        assert_eq!(sender.0.lock().unwrap().len(), 1);
        assert_eq!(
            get_offset(&db, "thread-a").unwrap().unwrap().cursor,
            i64::try_from(bytes.len()).unwrap()
        );
    }
}
