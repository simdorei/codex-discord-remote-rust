use cdr_codex_state::{CodexThreadStore, load_session_thread_names, resolve_state_db_path};

#[test]
#[ignore = "requires CDR_LIVE_CODEX_HOME and reads the real Codex state read-only"]
fn current_codex_state_database_and_session_index_are_readable() {
    let home = std::env::var_os("CDR_LIVE_CODEX_HOME")
        .map(std::path::PathBuf::from)
        .expect("CDR_LIVE_CODEX_HOME must point to the current Codex home");
    let state_path = resolve_state_db_path(&home);
    let store = CodexThreadStore::open(&state_path).expect("open current state DB read-only");
    let threads = store.load_recent_threads(3).expect("read recent threads");
    assert!(!threads.is_empty(), "current state DB contained no threads");
    let _names = load_session_thread_names(&home.join("session_index.jsonl"))
        .expect("read current session index");
}
