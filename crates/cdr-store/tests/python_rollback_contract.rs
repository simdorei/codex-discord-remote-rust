use std::path::Path;
use std::process::Command;

use cdr_store::processed::{claim, mark};
use cdr_store::prompt_intake::{NewPromptIntake, admit_prompt_intake, get_prompt_intake};
use cdr_store::schema::open_initialized;

const FAR_FUTURE: f64 = 10_000_000_000.0;

#[test]
fn python_rollback_opens_rust_extended_store_without_conversion() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("rollback.sqlite");
    let conn = open_initialized(&path).expect("initialize Rust store");
    conn.execute(
        "INSERT INTO codex_delivery_outbox (\
            delivery_id, job_id, target_thread_id, turn_id, channel_id, content, \
            created_at, updated_at\
         ) VALUES ('delivery-a', 'job-a', 'thread-a', 'turn-a', 7, 'keep-me', 1, 1)",
        [],
    )
    .expect("insert Rust extension sentinel");
    drop(conn);
    admit_prompt_intake(
        &path,
        NewPromptIntake {
            job_id: "intake-a",
            target_thread_id: "thread-a",
            channel_id: 7,
            owner_user_id: Some(8),
            discord_message_id: Some(9),
            raw_prompt: "keep enriched raw prompt",
            auto_queue_when_busy: true,
            require_current_mirror: true,
            created_at: 1.0,
        },
    )
    .expect("insert prompt intake extension sentinel");

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = "import sqlite3,sys; from codex_discord_store_schema import init_store_schema; \
        c=sqlite3.connect(sys.argv[1]); init_store_schema(c); \
        assert c.execute('PRAGMA user_version').fetchone()[0] == 2; \
        c.execute('INSERT INTO discord_processed_messages VALUES (77, 12.5)'); c.commit(); c.close()";
    let output = python_command(&repo)
        .current_dir(repo)
        .args(["-c", script])
        .arg(&path)
        .output()
        .expect("run Python rollback probe");
    assert!(
        output.status.success(),
        "Python rollback probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let conn = open_initialized(&path).expect("Rust reopens Python-used store");
    let processed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM discord_processed_messages WHERE message_id = 77",
            [],
            |row| row.get(0),
        )
        .expect("Python sentinel remains");
    let delivery: String = conn
        .query_row(
            "SELECT content FROM codex_delivery_outbox WHERE delivery_id = 'delivery-a'",
            [],
            |row| row.get(0),
        )
        .expect("Rust extension sentinel remains");
    assert_eq!(processed, 1);
    assert_eq!(delivery, "keep-me");
    drop(conn);
    let intake = get_prompt_intake(&path, "intake-a")
        .expect("read prompt intake after Python rollback")
        .expect("prompt intake remains");
    assert_eq!(intake.raw_prompt, "keep enriched raw prompt");
    assert_eq!(intake.discord_message_id, Some(9));
}

#[test]
fn rust_replay_barriers_survive_far_future_python_cleanup_and_block_python_reclaim() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("rust-to-python.sqlite");
    assert!(claim(&path, 801, 1.0).expect("insert Rust claim-only barrier"));
    assert!(claim(&path, 802, 2.0).expect("insert Rust marked barrier"));
    mark(&path, 802, 3.0).expect("mark Rust barrier");

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = "import sys; from pathlib import Path; \
        from codex_discord_store_processed_messages import cleanup_processed_discord_messages, claim_persistent_discord_message_id; \
        p=Path(sys.argv[1]); \
        assert cleanup_processed_discord_messages(p, retention_seconds=1.0, now=10000000000.0) == 0; \
        assert not claim_persistent_discord_message_id(p, 801, now=10000000001.0); \
        assert not claim_persistent_discord_message_id(p, 802, now=10000000001.0)";
    assert_python_success(&repo, &path, script, "Rust-to-Python replay barrier probe");

    assert!(!claim(&path, 801, FAR_FUTURE).expect("Rust still rejects claim-only duplicate"));
    assert!(!claim(&path, 802, FAR_FUTURE).expect("Rust still rejects marked duplicate"));
}

#[test]
fn python_replay_barriers_survive_far_future_python_cleanup_and_block_rust_reclaim() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("python-to-rust.sqlite");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = "import sys; from pathlib import Path; \
        from codex_discord_store_processed_messages import cleanup_processed_discord_messages, claim_persistent_discord_message_id, mark_processed_discord_message_id; \
        p=Path(sys.argv[1]); \
        assert claim_persistent_discord_message_id(p, 811, now=1.0); \
        assert claim_persistent_discord_message_id(p, 812, now=2.0); \
        mark_processed_discord_message_id(p, 812, now=3.0); \
        assert cleanup_processed_discord_messages(p, retention_seconds=1.0, now=10000000000.0) == 0";
    assert_python_success(&repo, &path, script, "Python-to-Rust replay barrier probe");

    assert!(!claim(&path, 811, FAR_FUTURE).expect("Rust rejects Python claim-only duplicate"));
    assert!(!claim(&path, 812, FAR_FUTURE).expect("Rust rejects Python marked duplicate"));
}

#[test]
fn python_mark_only_initializes_exact_v2_row_and_blocks_rust_reclaim() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("python-mark-only.sqlite");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = "import sys; from pathlib import Path; \
        from codex_discord_store_processed_messages import mark_processed_discord_message_id; \
        mark_processed_discord_message_id(Path(sys.argv[1]), 821, now=12.5)";
    assert_python_success(&repo, &path, script, "Python mark-only fresh-store probe");

    let connection = rusqlite::Connection::open(&path).expect("open Python-created store");
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .expect("read Python-created schema version");
    assert_eq!(version, 2);

    let mut statement = connection
        .prepare("PRAGMA table_info(discord_processed_messages)")
        .expect("inspect processed-message schema");
    let columns = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .expect("query processed-message schema")
        .collect::<Result<Vec<_>, _>>()
        .expect("read processed-message schema");
    assert_eq!(
        columns,
        vec![
            (0, "message_id".to_owned(), "INTEGER".to_owned(), 0, None, 1),
            (1, "seen_at".to_owned(), "REAL".to_owned(), 1, None, 0),
        ]
    );

    drop(statement);
    let mut statement = connection
        .prepare("SELECT message_id, seen_at FROM discord_processed_messages ORDER BY message_id")
        .expect("query Python-marked rows");
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)))
        .expect("query Python-marked rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("read Python-marked rows");
    assert_eq!(rows, vec![(821, 12.5)]);
    drop(statement);
    drop(connection);

    assert!(!claim(&path, 821, FAR_FUTURE).expect("Rust rejects Python mark-only duplicate"));
}

#[test]
fn python_cleanup_compatibility_noop_still_initializes_schema() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("cleanup-init.sqlite");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = "import sqlite3,sys; from pathlib import Path; \
        from codex_discord_store_processed_messages import cleanup_processed_discord_messages; \
        p=Path(sys.argv[1]); \
        assert cleanup_processed_discord_messages(p, retention_seconds=1.0, now=10000000000.0) == 0; \
        c=sqlite3.connect(p); \
        assert c.execute('PRAGMA user_version').fetchone()[0] == 2; \
        assert c.execute(\"SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='discord_processed_messages'\").fetchone()[0] == 1; \
        c.close()";
    assert_python_success(
        &repo,
        &path,
        script,
        "Python cleanup schema initialization probe",
    );
}

fn assert_python_success(repo: &Path, path: &Path, script: &str, context: &str) {
    let output = python_command(repo)
        .current_dir(repo)
        .args(["-c", script])
        .arg(path)
        .output()
        .expect("run Python replay barrier probe");
    assert!(
        output.status.success(),
        "{context} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn python_command(repo: &Path) -> Command {
    if let Some(executable) = std::env::var_os("PYTHON_EXE").filter(|value| !value.is_empty()) {
        return Command::new(executable);
    }
    if cfg!(windows) {
        let portable = repo.join(".python-portable/python.exe");
        if portable.is_file() {
            return Command::new(portable);
        }
        let mut command = Command::new("py");
        command.arg("-3");
        command
    } else {
        let local = repo.join("remote_mcp_server/.venv/bin/python");
        if local.is_file() {
            Command::new(local)
        } else {
            Command::new("python3")
        }
    }
}
