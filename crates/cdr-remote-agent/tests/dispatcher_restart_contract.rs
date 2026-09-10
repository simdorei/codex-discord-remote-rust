use std::fs;

use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_agent::restart_handoff::RestartProject;
use chrono::{Duration, TimeZone, Utc};

#[tokio::test]
async fn dispatcher_restart_snapshot_restores_exact_active_bindings() {
    let directory = tempfile::tempdir().unwrap();
    let root_a = directory.path().join("a");
    let root_b = directory.path().join("b");
    fs::create_dir(&root_a).unwrap();
    fs::create_dir(&root_b).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 8, 31, 1, 2, 3).unwrap();
    let source = LocalProjectDispatcher::new();
    source
        .upsert("thread-b", &root_b, now + Duration::minutes(10))
        .await
        .unwrap();
    source
        .upsert("thread-a", &root_a, now + Duration::minutes(5))
        .await
        .unwrap();

    let snapshot = source.restart_projects(now).await;
    assert_eq!(
        snapshot,
        vec![
            RestartProject {
                thread_id: "thread-a".into(),
                root: root_a.canonicalize().unwrap(),
                expires_at: now + Duration::minutes(5),
            },
            RestartProject {
                thread_id: "thread-b".into(),
                root: root_b.canonicalize().unwrap(),
                expires_at: now + Duration::minutes(10),
            },
        ]
    );

    let restored = LocalProjectDispatcher::new();
    restored.restore_restart_projects(&snapshot).await.unwrap();
    assert_eq!(restored.restart_projects(now).await, snapshot);
}

#[tokio::test]
async fn dispatcher_restart_snapshot_omits_expired_bindings() {
    let directory = tempfile::tempdir().unwrap();
    let now = Utc.with_ymd_and_hms(2026, 8, 31, 1, 2, 3).unwrap();
    let dispatcher = LocalProjectDispatcher::new();
    dispatcher
        .upsert("expired", directory.path(), now - Duration::seconds(1))
        .await
        .unwrap();

    assert!(dispatcher.restart_projects(now).await.is_empty());
}
