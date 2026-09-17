use cdr_codex_state::CodexThreadStore;
use rusqlite::{Connection, params};

#[test]
fn mirror_roots_include_interactive_sources_not_internal_or_archived_threads() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE threads (id TEXT PRIMARY KEY,title TEXT,cwd TEXT,updated_at INTEGER,
         rollout_path TEXT,model TEXT,reasoning_effort TEXT,tokens_used INTEGER,
         archived INTEGER,archived_at INTEGER,source TEXT,thread_source TEXT);",
    )
    .unwrap();
    for (id, source, provenance, archived, title) in [
        ("a-cli", "cli", Some("user"), 0, "CLI"),
        ("b-app", "app-server", Some("user"), 0, "App"),
        ("c-api", "appServer", Some(""), 0, "API legacy"),
        ("d-vscode", "vscode", None, 0, "VS Code"),
        ("internal", "app-server", Some("subagent"), 0, "Internal"),
        ("exec", "exec", Some("user"), 0, "Execution"),
        ("old", "cli", Some("user"), 1, "Archived"),
        ("untitled", "app-server", Some("user"), 0, ""),
    ] {
        db.execute(
            "INSERT INTO threads VALUES (?,?,'C:/repo',10,'missing.jsonl','model','high',0,?,20,?,?)",
            params![id, title, archived, source, provenance],
        ).unwrap();
    }
    let reader = CodexThreadStore::open(&path).unwrap();
    let ids = |threads: Vec<cdr_codex_state::ThreadInfo>| {
        threads.into_iter().map(|t| t.id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(reader.load_mirror_root_threads(0).unwrap()),
        ["a-cli", "b-app", "c-api", "d-vscode"]
    );
    assert_eq!(
        ids(reader.load_mirror_root_threads(2).unwrap()),
        ["a-cli", "b-app"]
    );
    assert_eq!(
        ids(reader.load_user_root_threads(0).unwrap()),
        ["d-vscode"],
        "legacy reader remains unchanged"
    );
    assert_eq!(
        reader.load_recent_threads(0).unwrap().len(),
        7,
        "plain list does not inherit mirror-root filtering"
    );
    assert_eq!(ids(reader.load_archived_threads(0).unwrap()), ["old"]);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        8
    );
}

#[test]
fn inventory_schema_errors_are_not_reported_as_empty_lists() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE threads (id TEXT)").unwrap();
    let reader = CodexThreadStore::open(&path).unwrap();
    assert!(reader.load_mirror_root_threads(0).is_err());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('threads')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
