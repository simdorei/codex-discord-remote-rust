use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use std::sync::Arc;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn doctor_reports_read_failures_without_creating_missing_files_or_exposing_values() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
    let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
    let backend = Arc::new(target::FakeBackend::default());
    let executor = target::executor(&root, db.clone(), bridge.clone(), backend.clone());
    let first = executor
        .execute(CommandAction::Doctor, 42, 3)
        .await
        .unwrap()
        .text;
    assert!(first.contains("codex_protocol_registration:"), "{first}");
    assert!(first.contains("앱 열기 성공을 뜻하지 않음"), "{first}");
    assert!(
        first.contains("state_db: read OK") && first.contains("threads=3"),
        "{first}"
    );
    assert!(
        first.contains("bridge_state: 조회 실패") && first.contains("session_index: 조회 실패")
    );
    assert!(!bridge.path().exists() && !root.path().join("session_index.jsonl").exists());
    let payload = b"{\"token\":\"PRIVATE_FIXTURE_SENTINEL\"}";
    std::fs::write(bridge.path(), payload).unwrap();
    let second = executor
        .execute(CommandAction::Doctor, 42, 3)
        .await
        .unwrap()
        .text;
    assert!(
        second.contains("readable JSON object") && !second.contains("PRIVATE_FIXTURE_SENTINEL")
    );
    assert_eq!(std::fs::read(bridge.path()).unwrap(), payload);
    rusqlite::Connection::open(&db)
        .unwrap()
        .pragma_update(None, "user_version", 999)
        .unwrap();
    let third = executor
        .execute(CommandAction::Doctor, 42, 3)
        .await
        .unwrap()
        .text;
    assert!(third.contains("schema version 999"), "{third}");
    assert_eq!(
        rusqlite::Connection::open(&db)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        999
    );
    assert!(
        backend.starts.lock().await.is_empty()
            && backend.resumes.lock().await.is_empty()
            && backend.forks.lock().await.is_empty()
    );
}
