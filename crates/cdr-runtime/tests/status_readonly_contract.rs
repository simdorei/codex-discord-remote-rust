use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use std::{sync::Arc, time::Duration};
#[path = "support/display_server.rs"]
mod peer;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn server_status_preserves_partial_failures_and_never_resumes_or_changes_selection() {
    for (mode, expected) in [
        ("normal", "등록된 목표 없음"),
        ("goal_error", "fixture goal lookup failed"),
        ("goal_missing", "goal 응답 필드 누락"),
        ("wrong_id", "원본 대화 ID 불일치"),
        ("conflicting_id", "원본 대화 ID 불일치"),
        ("missing_nested_id", "원본 대화 ID 불일치"),
        ("goal_hang", "전체 조회 시간 3초 초과"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("mirror.sqlite");
        cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
        let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
        let backend = Arc::new(target::FakeBackend::default());
        let executor = target::executor(&root, db, bridge.clone(), backend.clone());
        let server = Arc::new(peer::start(root.path(), mode).await);
        let executor = executor.with_server(server.clone());
        let text = tokio::time::timeout(
            Duration::from_secs(5),
            executor.execute(CommandAction::Status { reference: None }, 42, 3),
        )
        .await
        .unwrap()
        .unwrap()
        .text;
        assert!(text.contains(expected), "{mode}: {text}");
        assert!(text.contains("Second title") && text.contains("tokens_used: 50"));
        assert!(
            text.contains("조회 실패"),
            "missing rollout must remain visible: {text}"
        );
        let invalid_identity = matches!(mode, "wrong_id" | "conflicting_id" | "missing_nested_id");
        assert_eq!(text.contains("idle (작업 없음)"), !invalid_identity);
        if invalid_identity {
            let list = executor
                .execute(CommandAction::List { limit: 10 }, 42, 3)
                .await
                .unwrap()
                .text;
            assert!(list.contains("원본 대화 ID 불일치"), "{list}");
            assert!(!list.contains("idle (작업 없음)"), "{list}");
        }
        assert!(!bridge.path().exists());
        assert!(backend.starts.lock().await.is_empty() && backend.resumes.lock().await.is_empty());
        assert!(backend.forks.lock().await.is_empty());
        server.close().await.unwrap();
        let raw = std::fs::read_to_string(root.path().join("display-rpc.jsonl")).unwrap();
        let methods = raw
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(methods.iter().all(|v| matches!(
            v["method"].as_str(),
            Some("initialize" | "thread/read" | "thread/goal/get")
        )));
        assert_eq!(
            methods
                .iter()
                .filter(|v| v["method"] == "thread/read")
                .count(),
            if invalid_identity { 3 } else { 1 }
        );
        assert!(
            methods
                .iter()
                .filter(|v| v["method"] == "thread/read")
                .all(|v| v["params"]["includeTurns"] == false)
        );
    }
}
