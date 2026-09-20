#![cfg(windows)]
use std::{path::Path, process::Command};

#[test]
fn forced_cutover_recovers_owned_phases_and_backs_up_late_wal_writes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let db = root.join("discord_mirror.sqlite");
    let connection = cdr_store::schema::open_initialized(&db).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE cutover_sentinel(value TEXT);
        INSERT INTO cutover_sentinel VALUES ('before preliminary backup')",
        )
        .unwrap();
    let preliminary = cdr_store::backup::snapshot(&db).unwrap();
    connection
        .execute(
            "INSERT INTO cutover_sentinel VALUES ('late committed WAL input')",
            [],
        )
        .unwrap();
    std::fs::write(
        root.join(".env"),
        format!("CODEX_DISCORD_MIRROR_DB={}\n", db.display()),
    )
    .unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo.join("scripts/Test-CdrManualReserveCutover.ps1"))
        .arg("-FixtureRoot")
        .arg(root)
        .arg("-NativeBinary")
        .arg(env!("CARGO_BIN_EXE_cdr-runtime"))
        .env_remove("CODEX_DISCORD_ROOT")
        .env_remove("CODEX_DISCORD_MIRROR_DB")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("manual_reserve_cutover_tests_passed")
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let authoritative = std::fs::read_to_string(root.join("backup-result.txt")).unwrap();
    let count = |path: &Path| {
        rusqlite::Connection::open(path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM cutover_sentinel", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
    };
    assert_eq!(count(&preliminary), 1);
    assert_eq!(count(Path::new(&authoritative)), 2);
    assert_eq!(count(&db), 2, "recovery must never rewind current data");
}
