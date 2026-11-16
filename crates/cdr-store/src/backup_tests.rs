use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant};

use rusqlite::{Connection, ErrorCode};

use super::copy_snapshot;

#[test]
fn online_backup_waits_for_a_competing_writer_and_keeps_its_commit() {
    let temp = tempfile::tempdir().unwrap();
    let source_path = temp.path().join("source.sqlite");
    let target_path = temp.path().join("backup.sqlite");
    let writer = Connection::open(&source_path).unwrap();
    writer
        .execute_batch("CREATE TABLE facts(value); INSERT INTO facts VALUES(1);")
        .unwrap();
    let source = Connection::open(&source_path).unwrap();
    source.busy_timeout(Duration::ZERO).unwrap();
    writer
        .execute_batch("BEGIN EXCLUSIVE; UPDATE facts SET value=2;")
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let (tx, rx) = mpsc::channel();
    let worker_target = target_path.clone();
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        tx.send(copy_snapshot(
            &source,
            &worker_target,
            Duration::from_secs(3),
        ))
        .unwrap();
    });
    barrier.wait();
    let early = rx.recv_timeout(Duration::from_millis(150));
    writer.execute_batch("COMMIT").unwrap();
    worker.join().unwrap();
    assert!(
        matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
        "a temporary writer lock must not abort an online backup: {early:?}"
    );
    rx.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    let copied = Connection::open(&target_path).unwrap();
    assert_eq!(
        copied
            .query_row("SELECT value FROM facts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn online_backup_stops_at_its_budget_without_releasing_another_writer() {
    let temp = tempfile::tempdir().unwrap();
    let source_path = temp.path().join("source.sqlite");
    let writer = Connection::open(&source_path).unwrap();
    writer
        .execute_batch("CREATE TABLE facts(value); INSERT INTO facts VALUES(1);")
        .unwrap();
    let source = Connection::open(&source_path).unwrap();
    source.busy_timeout(Duration::ZERO).unwrap();
    writer
        .execute_batch("BEGIN EXCLUSIVE; UPDATE facts SET value=2;")
        .unwrap();
    let budget = Duration::from_millis(100);
    let began = Instant::now();
    let error = copy_snapshot(&source, &temp.path().join("backup.sqlite"), budget).unwrap_err();
    assert_eq!(error.sqlite_error_code(), Some(ErrorCode::DatabaseBusy));
    assert!(began.elapsed() >= budget);
    assert!(began.elapsed() < Duration::from_secs(2));
    assert!(!writer.is_autocommit());
    writer.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        writer
            .query_row("SELECT value FROM facts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn online_backup_finishes_during_continuous_committed_writes() {
    let temp = tempfile::tempdir().unwrap();
    let source_path = temp.path().join("source.sqlite");
    let target_path = temp.path().join("backup.sqlite");
    let writer = Connection::open(&source_path).unwrap();
    writer.execute_batch("CREATE TABLE facts(value INTEGER, payload BLOB); INSERT INTO facts VALUES(0, zeroblob(25165824));").unwrap();
    let source = Connection::open(&source_path).unwrap();
    source.busy_timeout(Duration::ZERO).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let (started_tx, started_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        writer
            .execute("UPDATE facts SET value=value+1", [])
            .unwrap();
        started_tx.send(()).unwrap();
        while !writer_stop.load(Ordering::Acquire) {
            writer
                .execute("UPDATE facts SET value=value+1", [])
                .unwrap();
        }
    });
    started_rx.recv().unwrap();
    let outcome = copy_snapshot(&source, &target_path, Duration::from_secs(3));
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
    outcome.unwrap();
    let copied = Connection::open(target_path).unwrap();
    let (value, size): (i64, i64) = copied
        .query_row("SELECT value, length(payload) FROM facts", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert!(value > 0);
    assert_eq!(size, 25_165_824);
    assert_eq!(
        copied
            .pragma_query_value(None, "integrity_check", |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}
