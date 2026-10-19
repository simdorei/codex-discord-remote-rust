//! Local temporary-store diagnostics; never opens an operational database.
use std::time::Instant;

fn main() {
    let temp = tempfile::tempdir().expect("temporary diagnostic directory");
    let path = temp.path().join("profile.sqlite");
    let first = Instant::now();
    drop(cdr_store::schema::open_initialized(&path).expect("initialize fixture"));
    let first_ms = first.elapsed().as_millis();
    let mut initialized_us = Vec::new();
    let mut raw_us = Vec::new();
    let mut write_us = Vec::new();
    for i in 0..100 {
        let start = Instant::now();
        drop(cdr_store::schema::open_initialized(&path).expect("reopen fixture"));
        initialized_us.push(start.elapsed().as_micros());
        let start = Instant::now();
        let db = rusqlite::Connection::open(&path).expect("raw fixture open");
        let _: i64 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        drop(db);
        raw_us.push(start.elapsed().as_micros());
        let start = Instant::now();
        assert_eq!(
            cdr_store::delivery_receipt::begin(&path, &format!("profile-{i}"), "fixture").unwrap(),
            cdr_store::delivery_receipt::ReceiptState::New,
        );
        write_us.push(start.elapsed().as_micros());
    }
    let backups =
        std::fs::read_dir(temp.path().join(".codex-discord-backups")).map_or(0, Iterator::count);
    println!(
        "{}",
        serde_json::json!({
            "temporary_database_only": true,
            "initial_ms": first_ms,
            "initialized_open_us": initialized_us,
            "raw_open_us": raw_us,
            "receipt_begin_us": write_us,
            "migration_backups": backups,
        })
    );
}
