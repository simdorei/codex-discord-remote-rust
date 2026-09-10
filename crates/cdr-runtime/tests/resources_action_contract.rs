use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use std::sync::Arc;
#[path = "support/action_target.rs"]
mod target;

#[cfg(windows)]
#[tokio::test]
async fn actual_resources_action_includes_host_measurements() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
    let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
    let backend = Arc::new(target::FakeBackend::default());
    let executor = target::executor(&root, db, bridge.clone(), backend.clone());
    let result = executor
        .execute(CommandAction::Resources, 42, 3)
        .await
        .unwrap();
    assert!(
        result.text.contains("CPU:")
            && result.text.contains("RAM:")
            && result.text.contains("Disk:"),
        "{}",
        result.text
    );
    assert!(result.text.contains("GiB") && result.text.contains("CPU sample"));
    assert!(!bridge.path().exists());
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
}
