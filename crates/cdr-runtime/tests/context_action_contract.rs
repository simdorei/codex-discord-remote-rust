use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use std::sync::Arc;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn context_reads_last_input_and_refreshes_actual_recent_text_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
    let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
    let backend = Arc::new(target::FakeBackend::default());
    let executor = target::executor(&root, db, bridge.clone(), backend.clone());
    let path = root.path().join("b.jsonl");
    let mut raw = concat!(
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-b\"}}\n",
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-09-07T10:00:00Z\",\"payload\":{\"type\":\"token_count\",\"info\":{\"model_context_window\":200000,\"last_token_usage\":{\"input_tokens\":80000},\"total_token_usage\":{\"input_tokens\":100000000}}}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"원래 요청\"}}\n"
    ).to_owned();
    std::fs::write(&path, &raw).unwrap();
    rusqlite::Connection::open(root.path().join("state.sqlite"))
        .unwrap()
        .execute(
            "UPDATE threads SET rollout_path=?,tokens_used=100000000 WHERE id='thread-b'",
            [path.to_string_lossy().as_ref()],
        )
        .unwrap();
    let command = || CommandAction::Context {
        all_threads: false,
        refresh: true,
        limit: 5,
    };
    let first = executor.execute(command(), 42, 3).await.unwrap();
    assert!(first.text.contains("last_input: 80000"));
    assert!(first.text.contains("used: 100000000"));
    assert!(first.text.contains("원래 요청"));
    assert!(!first.text.contains("recent_events:"));
    raw.push_str("{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final\",\"content\":[{\"type\":\"output_text\",\"text\":\"새 완료 답변\"}]}}\n");
    std::fs::write(&path, &raw).unwrap();
    let second = executor.execute(command(), 42, 3).await.unwrap();
    assert!(second.text.contains("새 완료 답변") && second.text.contains("assistant final"));
    std::fs::write(
        &path,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"other-thread\"}}\n",
    )
    .unwrap();
    let failed = executor.execute(command(), 42, 3).await.unwrap();
    assert!(failed.text.contains("조회 실패"));
    assert!(!failed.text.contains("last_input: 80000"));
    assert!(!failed.text.contains("새 완료 답변"));
    assert!(!bridge.path().exists());
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
}
