use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use std::sync::Arc;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn all_thread_refresh_caps_files_and_recent_text_without_changing_threads() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("mirror.sqlite");
    cdr_store::schema::open_initialized(&db).unwrap();
    let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
    let backend = Arc::new(target::FakeBackend::default());
    let executor = target::executor(&root, db, bridge.clone(), backend.clone());
    let state = rusqlite::Connection::open(root.path().join("state.sqlite")).unwrap();
    for i in 0..51 {
        let id = format!("budget-{i:02}");
        let path = root.path().join(format!("{id}.jsonl"));
        let meta = serde_json::json!({"type":"session_meta","payload":{"id":id}});
        let text = serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"한".repeat(1500)}});
        std::fs::write(&path, format!("{meta}\n{text}\n")).unwrap();
        state.execute("INSERT INTO threads SELECT ?1,title,cwd,?2,?3,model,reasoning_effort,tokens_used,archived,archived_at,source,thread_source FROM threads WHERE id='thread-b'",
            rusqlite::params![id, 100 + i, path.to_str().unwrap()]).unwrap();
    }
    let before: i64 = state
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .unwrap();
    let text = executor
        .execute(
            CommandAction::Context {
                all_threads: true,
                refresh: true,
                limit: 51,
            },
            42,
            3,
        )
        .await
        .unwrap()
        .text;
    assert!(text.contains("미조회 대화: 1"), "{text}");
    assert!(text.contains("budget-50") && !text.contains("budget-00"));
    let characters = text
        .lines()
        .filter_map(|line| line.strip_prefix("[user] "))
        .map(|line| line.chars().count())
        .sum::<usize>();
    assert_eq!(characters, 12_000);
    assert!(text.contains("이후 내용 생략"));
    let after: i64 = state
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after);
    assert!(!bridge.path().exists());
    assert!(
        backend.starts.lock().await.is_empty()
            && backend.resumes.lock().await.is_empty()
            && backend.forks.lock().await.is_empty()
    );
}
