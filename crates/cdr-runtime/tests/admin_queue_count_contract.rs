use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--admin", "active-queue-count", "--repo-root"])
        .arg(root)
        .env_clear()
        .output()
        .unwrap()
}
#[test]
fn active_count_reads_only_starting_and_running_without_mutating_legacy_store() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("discord_mirror.sqlite");
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TABLE codex_turn_queue(state TEXT); INSERT INTO codex_turn_queue VALUES('starting'),('running'),('pending'); PRAGMA user_version=2;").unwrap();
    let before = fs::read(&path).unwrap();
    let output = run(root.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "2");
    assert_eq!(fs::read(path).unwrap(), before);
}
#[test]
fn unavailable_count_is_an_error_not_an_idle_claim_or_empty_database() {
    let root = tempfile::tempdir().unwrap();
    let output = run(root.path());
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("unknown admin command"));
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
