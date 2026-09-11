use cdr_runtime::admin::attachment::target;
use rusqlite::{Connection, params};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
const FIRST: &str = "00000000-0000-0000-0000-000000000001";
const SECOND: &str = "00000000-0000-0000-0000-000000000002";
const ARCHIVED: &str = "11111111-1111-1111-1111-111111111111";
fn seed(root: &Path) -> (PathBuf, PathBuf) {
    let state = root.join("state.sqlite");
    let mirror = root.join("mirror.sqlite");
    let db = Connection::open(&state).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT,title TEXT,cwd TEXT,updated_at INTEGER,rollout_path TEXT,model TEXT,reasoning_effort TEXT,tokens_used INTEGER,archived_at INTEGER,archived INTEGER);").unwrap();
    for (id, age, archived, cwd) in [
        (FIRST, 2, 0, "/project/repo"),
        (SECOND, 1, 0, "/project/repo"),
        (ARCHIVED, 3, 1, "/project/archive"),
    ] {
        db.execute(
            "INSERT INTO threads VALUES(?,'title',?,?,'rollout','model','high',0,3,?)",
            params![id, cwd, age, archived],
        )
        .unwrap();
    }
    let db = Connection::open(&mirror).unwrap();
    db.execute_batch("CREATE TABLE mirror_threads(codex_thread_id TEXT PRIMARY KEY, discord_thread_id INTEGER); PRAGMA user_version=2;").unwrap();
    for (id, channel) in [(FIRST, 11), (SECOND, 12), (ARCHIVED, 13)] {
        db.execute(
            "INSERT INTO mirror_threads VALUES(?,?)",
            params![id, channel],
        )
        .unwrap();
    }
    (state, mirror)
}
#[test]
fn active_alias_exact_id_and_archived_lookup_preserve_database_bytes() {
    let root = tempfile::tempdir().unwrap();
    let (state, mirror) = seed(root.path());
    let before = (fs::read(&state).unwrap(), fs::read(&mirror).unwrap());
    for (reference, channel) in [
        (FIRST, "11"),
        ("repo:2", "12"),
        (ARCHIVED, "13"),
        ("archive", "13"),
    ] {
        let result = target::resolve_databases(&state, &mirror, reference).unwrap();
        assert_eq!(result.channel_id, channel);
        assert!(result.mirrored);
    }
    assert_eq!(
        before,
        (fs::read(&state).unwrap(), fs::read(&mirror).unwrap())
    );
}
#[test]
fn unknown_ambiguous_stale_and_duplicate_mapping_never_choose_another_room() {
    let root = tempfile::tempdir().unwrap();
    let (state, mirror) = seed(root.path());
    assert!(
        target::resolve_databases(&state, &mirror, "missing")
            .unwrap_err()
            .contains("not in active/archived threads")
    );
    assert!(
        target::resolve_databases(&state, &mirror, "repo")
            .unwrap_err()
            .contains("Multiple threads")
    );
    let db = Connection::open(&mirror).unwrap();
    db.execute(
        "UPDATE mirror_threads SET discord_thread_id=12 WHERE codex_thread_id=?",
        [FIRST],
    )
    .unwrap();
    assert!(
        target::resolve_databases(&state, &mirror, SECOND)
            .unwrap_err()
            .contains("multiple Codex threads")
    );
    db.execute(
        "DELETE FROM mirror_threads WHERE codex_thread_id=?",
        [SECOND],
    )
    .unwrap();
    assert!(
        target::resolve_databases(&state, &mirror, SECOND)
            .unwrap_err()
            .contains("run !mirror check, then !mirror sync")
    );
}
#[test]
fn configured_databases_require_no_python_codex_executable_or_home_and_missing_db_is_not_created() {
    let root = tempfile::tempdir().unwrap();
    let (state, mirror) = seed(root.path());
    let env = BTreeMap::from([
        ("CODEX_STATE_DB".into(), state.display().to_string()),
        (
            "CODEX_DISCORD_MIRROR_DB".into(),
            mirror.display().to_string(),
        ),
    ]);
    assert_eq!(
        target::resolve(&env, root.path(), SECOND)
            .unwrap()
            .channel_id,
        "12"
    );
    let missing = root.path().join("missing.sqlite");
    assert!(target::resolve_databases(&state, &missing, SECOND).is_err());
    assert!(!missing.exists());
}
