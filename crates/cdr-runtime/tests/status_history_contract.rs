use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use serde_json::json;
use std::sync::Arc;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn status_preserves_latest_repeated_final_and_filters_before_limiting() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
    let backend = Arc::new(target::FakeBackend::default());
    let executor = target::executor(
        &root,
        db,
        Arc::new(BridgeState::new(root.path().join("bridge.json"))),
        backend.clone(),
    );
    let path = root.path().join("history.jsonl");
    let mut events = vec![json!({"type":"session_meta","payload":{"id":"thread-b"}})];
    events
        .push(json!({"type":"event_msg","payload":{"type":"user_message","message":"원래 요청"}}));
    for (turn, text) in [
        ("t1", "승인되었습니다"),
        ("t2", "수정이 필요합니다"),
        ("t3", "승인되었습니다"),
    ] {
        events.push(json!({"type":"event_msg","payload":{"type":"agent_message","phase":"final","turn_id":turn,"message":text}}));
    }
    for i in 0..6 {
        events.push(json!({"type":"event_msg","payload":{"type":"agent_message","phase":"commentary","message":format!("중간 기록 {i}")}}));
    }
    events.push(json!({"type":"event_msg","payload":{"type":"agent_message","phase":"analysis","message":"분석 제외"}}));
    events.push(json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input"}}));
    events.push(json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"sandbox_permissions\":\"require_escalated\"}"}}));
    let raw = events
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&path, raw).unwrap();
    rusqlite::Connection::open(root.path().join("state.sqlite"))
        .unwrap()
        .execute(
            "UPDATE threads SET rollout_path=? WHERE id='thread-b'",
            [path.to_str().unwrap()],
        )
        .unwrap();
    let text = executor
        .execute(CommandAction::Status { reference: None }, 42, 3)
        .await
        .unwrap()
        .text;
    let history = text
        .lines()
        .filter(|line| line.starts_with('['))
        .collect::<Vec<_>>();
    assert_eq!(
        history,
        [
            "[user] 원래 요청",
            "[assistant final] 승인되었습니다",
            "[assistant final] 수정이 필요합니다",
            "[assistant final] 승인되었습니다"
        ]
    );
    assert!(
        !text.contains("중간 기록") && !text.contains("분석 제외") && !text.contains("required]")
    );
    assert!(backend.starts.lock().await.is_empty() && backend.forks.lock().await.is_empty());
}
