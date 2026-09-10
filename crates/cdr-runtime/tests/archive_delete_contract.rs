use std::fs;
use std::path::Path;

use cdr_runtime::archive_delete::{ArchiveDeleteError, ArchiveDeletePaths, delete_archived_thread};
use rusqlite::Connection;
use serde_json::{Value, json};
#[path = "support/archive_delete_recovery.rs"]
mod recovery;

fn paths(root: &Path) -> ArchiveDeletePaths {
    ArchiveDeletePaths {
        state_db: root.join("state.sqlite"),
        log_db: root.join("logs.sqlite"),
        global_state: root.join("global.json"),
        bridge_state: root.join("bridge.json"),
        session_index: root.join("session_index.jsonl"),
        archived_sessions: root.join("archived_sessions"),
        backup_root: root.join("maintenance_backups"),
    }
}

fn seed(paths: &ArchiveDeletePaths, rollout: &Path, archived: bool) {
    fs::create_dir_all(&paths.archived_sessions).unwrap();
    if let Some(parent) = rollout.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(rollout, b"rollout\n").unwrap();
    let state = Connection::open(&paths.state_db).unwrap();
    state
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER, archived_at INTEGER);\
             CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT);",
        )
        .unwrap();
    state
        .execute(
            "INSERT INTO threads VALUES (?1,'Old','C:/repo',1,?2,'gpt','high',0,?3,2)",
            (
                "thread-old",
                rollout.to_string_lossy().as_ref(),
                i64::from(archived),
            ),
        )
        .unwrap();
    state
        .execute(
            "INSERT INTO thread_spawn_edges VALUES ('parent','thread-old')",
            [],
        )
        .unwrap();
    let logs = Connection::open(&paths.log_db).unwrap();
    logs.execute_batch("CREATE TABLE logs (thread_id TEXT, message TEXT);")
        .unwrap();
    logs.execute("INSERT INTO logs VALUES ('thread-old','secret')", [])
        .unwrap();
    fs::write(
        &paths.bridge_state,
        serde_json::to_vec(&json!({
            "selected_thread_id": "thread-old",
            "recent_live_approval_requests": {"thread-old": {"id": 1}},
            "recent_ui_thread": {"thread_id": "thread-old"},
            "keep": true
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &paths.global_state,
        serde_json::to_vec(&json!({
            "queued-follow-ups": {"thread-old": ["later"]},
            "pinned-thread-ids": ["thread-old", "thread-keep"]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &paths.session_index,
        "{\"id\":\"thread-old\"}\nnot-json\n{\"id\":\"thread-keep\"}\n",
    )
    .unwrap();
}

fn row_count(path: &Path, table: &str, thread_id: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE id = ?1"),
            [thread_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn confirmed_delete_backs_up_then_scrubs_every_local_reference() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let rollout = paths.archived_sessions.join("thread-old.jsonl");
    seed(&paths, &rollout, true);

    let result = delete_archived_thread(&paths, "thread-old").unwrap();

    assert_eq!(row_count(&paths.state_db, "threads", "thread-old"), 0);
    assert!(!rollout.exists());
    assert!(result.backup_dir.is_dir());
    let backup_state = result.backup_dir.join("state.sqlite");
    assert_eq!(row_count(&backup_state, "threads", "thread-old"), 1);
    let backup_rollout = result.backup_dir.join("transcript/rollout.jsonl");
    assert_eq!(fs::read(&backup_rollout).unwrap(), b"rollout\n");
    assert!(result.backup_paths.contains(&backup_rollout));
    // A backup is useful only if the original transcript can actually be recovered.
    let recovered = temp.path().join("recovered.jsonl");
    fs::copy(&backup_rollout, &recovered).unwrap();
    assert_eq!(fs::read(recovered).unwrap(), b"rollout\n");
    let bridge: Value = serde_json::from_slice(&fs::read(&paths.bridge_state).unwrap()).unwrap();
    assert_eq!(bridge, json!({"keep": true}));
    let global: Value = serde_json::from_slice(&fs::read(&paths.global_state).unwrap()).unwrap();
    assert_eq!(global["pinned-thread-ids"], json!(["thread-keep"]));
    assert!(global.get("queued-follow-ups").is_none());
    let index = fs::read_to_string(&paths.session_index).unwrap();
    assert_eq!(index, "not-json\n{\"id\":\"thread-keep\"}\n");
    let log_count: i64 = Connection::open(&paths.log_db)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE thread_id='thread-old'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(log_count, 0);
}

#[test]
fn outside_or_active_rollout_is_rejected_without_mutation() {
    for (outside, archived, expected) in [(true, true, "outside"), (false, false, "active")] {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(temp.path());
        let rollout = if outside {
            temp.path().join("outside.jsonl")
        } else {
            paths.archived_sessions.join("thread-old.jsonl")
        };
        seed(&paths, &rollout, archived);
        let error = delete_archived_thread(&paths, "thread-old").unwrap_err();
        assert!(error.to_string().contains(expected));
        assert_eq!(row_count(&paths.state_db, "threads", "thread-old"), 1);
        assert!(rollout.exists());
        assert!(!paths.backup_root.exists());
    }
}

#[test]
fn backup_failure_leaves_source_state_and_rollout_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let rollout = paths.archived_sessions.join("thread-old.jsonl");
    seed(&paths, &rollout, true);
    fs::write(&paths.backup_root, b"not a directory").unwrap();

    let error = delete_archived_thread(&paths, "thread-old").unwrap_err();

    assert!(matches!(error, ArchiveDeleteError::Io { .. }));
    assert_eq!(row_count(&paths.state_db, "threads", "thread-old"), 1);
    assert!(rollout.exists());
}

#[test]
fn missing_transcript_backup_refuses_deletion_before_any_state_change() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let rollout = paths.archived_sessions.join("thread-old.jsonl");
    seed(&paths, &rollout, true);
    fs::remove_file(&rollout).unwrap();
    let bridge_before = fs::read(&paths.bridge_state).unwrap();
    let error = delete_archived_thread(&paths, "thread-old").unwrap_err();
    assert!(matches!(error, ArchiveDeleteError::Io { .. }));
    assert_eq!(row_count(&paths.state_db, "threads", "thread-old"), 1);
    assert_eq!(fs::read(&paths.bridge_state).unwrap(), bridge_before);
}

#[test]
fn partial_failure_reports_stage_backup_and_original_error_without_auto_retry() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let rollout = paths.archived_sessions.join("thread-old.jsonl");
    seed(&paths, &rollout, true);
    Connection::open(&paths.log_db).unwrap().execute_batch(
        "CREATE TRIGGER reject_log_delete BEFORE DELETE ON logs BEGIN SELECT RAISE(ABORT, 'injected log delete failure'); END;"
    ).unwrap();
    let error = delete_archived_thread(&paths, "thread-old").unwrap_err();
    let message = error.to_string();
    let ArchiveDeleteError::Partial {
        last_completed_stage,
        backup_dir,
        source,
    } = error
    else {
        panic!("expected partial failure");
    };
    assert_eq!(last_completed_stage, "state row deletion");
    assert!(matches!(*source, ArchiveDeleteError::Sqlite { .. }));
    assert!(message.contains("injected log delete failure"));
    assert!(message.contains("Do not retry automatically"));
    assert!(backup_dir.join("transcript/rollout.jsonl").is_file());
    assert_eq!(
        row_count(&backup_dir.join("state.sqlite"), "threads", "thread-old"),
        1
    );
    assert!(rollout.exists());
}
