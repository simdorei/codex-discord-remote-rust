use std::{fs, process::Command};

use cdr_app_server::AppServerConfig;
use cdr_runtime::{
    admin::threads::{ThreadAdminAction, execute},
    runtime_paths::{PathInputs, RuntimePaths},
    soak::native_fixture,
};

fn fixture(root: &std::path::Path, scenario: &str) -> (RuntimePaths, AppServerConfig, String) {
    let id = uuid::Uuid::new_v4().to_string();
    let state = root.join("state.sqlite");
    let mirror = root.join("mirror.sqlite");
    let db = rusqlite::Connection::open(&state).unwrap();
    db.execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    db.execute("UPDATE threads SET id=? WHERE id='thread-b'", [&id])
        .unwrap();
    drop(db);
    cdr_store::schema::open_initialized(&mirror).unwrap();
    let environment = std::collections::BTreeMap::from([
        ("CODEX_HOME".into(), root.to_str().unwrap().into()),
        ("CODEX_STATE_DB".into(), state.to_str().unwrap().into()),
        (
            "CODEX_DISCORD_MIRROR_DB".into(),
            mirror.to_str().unwrap().into(),
        ),
        (
            "CODEX_EXE".into(),
            env!("CARGO_BIN_EXE_cdr-offline-soak").into(),
        ),
    ]);
    let paths = RuntimePaths::resolve(
        &environment,
        &PathInputs::new(root.into(), root.into(), vec![], vec![]),
    )
    .unwrap();
    let mut config = native_fixture::config("archive");
    config
        .environment
        .insert("ARCHIVE_STATE".into(), state.to_str().unwrap().into());
    config.environment.insert(
        "ARCHIVE_LOG".into(),
        root.join("rpc.jsonl").to_str().unwrap().into(),
    );
    config
        .environment
        .insert("ARCHIVE_SCENARIO".into(), scenario.into());
    (paths, config, id)
}

#[tokio::test]
async fn native_list_and_archive_use_the_same_persisted_target_and_guards_as_discord() {
    let root = tempfile::tempdir().unwrap();
    let (paths, config, id) = fixture(root.path(), "normal");
    let bridge = cdr_runtime::bridge_state::BridgeState::new(paths.bridge_state.clone());
    bridge.set_selected_thread_id(Some(&id)).unwrap();
    let listed = execute(&paths, ThreadAdminAction::List { limit: 0 }, config.clone())
        .await
        .unwrap();
    assert!(listed.contains(&id) && listed.contains("used "));
    let archived = execute(
        &paths,
        ThreadAdminAction::Archive {
            thread_id: id.clone(),
        },
        config,
    )
    .await
    .unwrap();
    assert!(
        archived.contains(&format!("Archived Codex thread {id}"))
            && archived.contains("persisted state verified")
    );
    assert_eq!(
        rusqlite::Connection::open(&paths.state_db)
            .unwrap()
            .query_row("SELECT archived FROM threads WHERE id=?", [&id], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(bridge.selected_thread_id().unwrap().is_none());
    let calls = fs::read_to_string(root.path().join("rpc.jsonl")).unwrap();
    assert!(!calls.contains("thread/fork") && !calls.contains("turn/start"));
    assert_eq!(calls.matches("\"thread/archive\"").count(), 1);
}

#[tokio::test]
async fn unsupported_writer_or_nonidle_status_is_not_bypassed_by_native_admin() {
    for scenario in ["active_without_event", "wrong_read_identity"] {
        let root = tempfile::tempdir().unwrap();
        let (paths, config, id) = fixture(root.path(), scenario);
        let error = execute(
            &paths,
            ThreadAdminAction::Archive {
                thread_id: id.clone(),
            },
            config,
        )
        .await
        .unwrap_err();
        assert!(error.contains("confirmed idle status"), "{error}");
        assert_eq!(
            rusqlite::Connection::open(&paths.state_db)
                .unwrap()
                .query_row("SELECT archived FROM threads WHERE id=?", [&id], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let calls = fs::read_to_string(root.path().join("rpc.jsonl")).unwrap();
        assert!(!calls.contains("thread/archive") && !calls.contains("thread/fork"));
    }
}
#[test]
fn archive_requires_exact_uuid_before_any_path_or_app_server_access() {
    let root = tempfile::tempdir().unwrap();
    for reference in ["1", "project-name", "", "thread-a"] {
        let out = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
            .args([
                "--admin",
                "archive-thread",
                "--thread-id",
                reference,
                "--repo-root",
            ])
            .arg(root.path())
            .env_clear()
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("exact UUID"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
#[test]
fn missing_project_store_is_not_created_or_reported_as_an_empty_list() {
    let root = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--admin", "list-threads", "--repo-root"])
        .arg(root.path())
        .env_clear()
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("unknown admin command"));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
