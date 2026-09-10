use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use std::sync::Arc;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn list_where_and_archived_display_sources_without_mutating_targets() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
    let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
    let backend = Arc::new(target::FakeBackend::default());
    let executor = target::executor(&root, db, bridge.clone(), backend.clone());
    let path = root.path().join("b.jsonl");
    std::fs::write(&path, concat!(
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-b\"}}\n",
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-09-07T10:00:00Z\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":80000}}}}\n"
    )).unwrap();
    rusqlite::Connection::open(root.path().join("state.sqlite"))
        .unwrap()
        .execute(
            "UPDATE threads SET rollout_path=?,tokens_used=100000000 WHERE id='thread-b'",
            [path.to_string_lossy().as_ref()],
        )
        .unwrap();
    let list = executor
        .execute(CommandAction::List { limit: 2 }, 42, 3)
        .await
        .unwrap()
        .text;
    assert!(list.contains("ctx 80.000K/80.000K"), "{list}");
    assert!(list.contains("used 100.000M") && list.contains("rec archive"));
    assert!(list.contains("state 미확인"));
    assert!(list.contains("마지막 저장") && list.contains("조회 실패"));
    assert!(list.contains("1 | alpha") && list.contains("2 | beta"));
    let location = executor
        .execute(CommandAction::Where, 42, 3)
        .await
        .unwrap()
        .text;
    assert!(location.contains("Second title") && location.contains("C:/repos/beta"));
    assert!(location.contains("mapped to this Discord room"));
    let status = executor
        .execute(CommandAction::Status { reference: None }, 42, 3)
        .await
        .unwrap()
        .text;
    assert!(status.contains("현재 실행 상태 미확인"), "{status}");
    assert!(status.contains("마지막 저장 model: gpt-5") && status.contains("last_input: 80000"));
    assert!(status.contains("tokens_used: 100000000"));
    let archived = executor
        .execute(CommandAction::ArchivedList { limit: 1 }, 42, 3)
        .await
        .unwrap()
        .text;
    assert!(
        archived.contains("archived_at: 1970-01-01T00:00:30+00:00"),
        "{archived}"
    );
    assert!(!bridge.path().exists());
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
}
