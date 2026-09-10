use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;

use cdr_codex_state::{
    CodexThreadStore, build_ui_name_prefixes, load_missing_vscode_rollout_threads,
    load_session_thread_names, normalize_ui_match_text, parse_rollout_thread,
    read_new_session_events,
};
use rusqlite::Connection;
use tempfile::tempdir;

const THREAD_ID: &str = "01234567-89ab-cdef-0123-456789abcdef";

#[test]
fn reads_active_user_and_archived_threads_from_existing_schema() {
    let temp = tempdir().expect("temp directory");
    let path = temp.path().join("state_5.sqlite");
    let connection = Connection::open(&path).expect("create state DB");
    connection
        .execute_batch(
            "CREATE TABLE threads (
            id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER,
            rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER,
            archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT
        );
        INSERT INTO threads VALUES
            ('active','Active','C:/repo',20,'active.jsonl','gpt-5','high',100,0,0,'vscode','user'),
            ('other','Other','C:/repo',10,'other.jsonl','gpt-5','low',50,0,0,'cli','user'),
            ('archived','Old','C:/old',5,'old.jsonl','gpt-4','medium',25,1,30,'vscode','user');",
        )
        .expect("state fixture");
    drop(connection);

    let store = CodexThreadStore::open(&path).expect("read-only store");
    assert_eq!(
        store.load_recent_threads(1).expect("recent")[0].id,
        "active"
    );
    let user = store.load_user_root_threads(0).expect("user root");
    assert_eq!(
        user.iter()
            .map(|thread| thread.id.as_str())
            .collect::<Vec<_>>(),
        ["active"]
    );
    let archived = store.load_archived_threads(20).expect("archived");
    assert_eq!(archived[0].archived_at, 30);
}

#[test]
fn session_index_and_rollout_parser_match_python_behavior() {
    let temp = tempdir().expect("temp directory");
    let index = temp.path().join("session_index.jsonl");
    fs::write(
        &index,
        format!("{{\"id\":\"{THREAD_ID}\",\"thread_name\":\"  Saved title  \"}}\nnot-json\n"),
    )
    .expect("session index fixture");
    let names = load_session_thread_names(&index).expect("session names");
    assert_eq!(names[THREAD_ID], "Saved title");

    let rollout = temp.path().join(format!("rollout-2026-{THREAD_ID}.jsonl"));
    fs::write(
        &rollout,
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"source\":\"vscode\",\"thread_source\":\"user\",\"cwd\":\"C:/repo\"}}\n",
            "{\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-5.6\",\"reasoning_effort\":\"high\"}}\n"
        ),
    )
    .expect("rollout fixture");
    let thread = parse_rollout_thread(&rollout, THREAD_ID, Some(&names))
        .expect("parse rollout")
        .expect("loadable vscode rollout");
    assert_eq!(thread.title, "Saved title");
    assert_eq!(thread.model, "gpt-5.6");

    let threads = load_missing_vscode_rollout_threads(temp.path(), &HashSet::new(), Some(&names))
        .expect("scan rollouts");
    assert_eq!(threads.len(), 1);
    let excluded = HashSet::from([THREAD_ID.to_owned()]);
    assert!(
        load_missing_vscode_rollout_threads(temp.path(), &excluded, Some(&BTreeMap::new()))
            .expect("excluded scan")
            .is_empty()
    );
}

#[test]
fn tail_reader_stops_before_partial_json_and_resumes_after_append() {
    let temp = tempdir().expect("temp directory");
    let path = temp.path().join("rollout.jsonl");
    fs::write(&path, "{\"id\":1}\n{\"id\":").expect("partial rollout");
    let first = read_new_session_events(&path, 0, None).expect("first tail");
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.next_offset, 9);

    let mut file = OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append rollout");
    writeln!(file, "2}}").expect("complete rollout");
    let second = read_new_session_events(&path, first.next_offset, None).expect("second tail");
    assert_eq!(second.events[0]["id"], 2);
}

#[test]
fn ui_name_normalization_and_prefixes_are_stable() {
    assert_eq!(
        normalize_ui_match_text("\r\n  hello   world \n later"),
        "hello world"
    );
    let text = "x".repeat(130);
    let prefixes = build_ui_name_prefixes(&text);
    assert_eq!(
        prefixes.iter().map(String::len).collect::<Vec<_>>(),
        [130, 120, 96, 72, 56, 40]
    );
}
