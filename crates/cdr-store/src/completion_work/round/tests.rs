use super::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rusqlite::Connection;

use crate::schema::checked_read::test_support::{self, Boundary, HookGuard};
use crate::{delivery, queue};

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.sqlite");
        drop(crate::schema::open_initialized(&path).unwrap());
        Self {
            _directory: directory,
            path,
        }
    }

    fn queued(&self, id: &str, channel: i64) {
        queue::enqueue(
            &self.path,
            queue::NewQueueJob {
                job_id: id,
                target_thread_id: id,
                channel_id: channel,
                owner_user_id: Some(1),
                discord_message_id: None,
                app_server_generation: 1,
                prompt: "fixture",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
    }

    fn completed(&self, id: &str, channel: i64, at: f64) {
        self.queued(id, channel);
        queue::begin_attempt(&self.path, id, &[], 1).unwrap();
        queue::mark_running(&self.path, id, "turn", 1).unwrap();
        delivery::stage_queue_completion(&self.path, id, "body", at).unwrap();
    }

    fn uncached_catalog(&self) -> Connection {
        let db = Connection::open(&self.path).unwrap();
        let _: String = db
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .unwrap();
        db.execute_batch(&format!(
            "CREATE TABLE metadata_round_marker_{} (value TEXT);",
            uuid::Uuid::new_v4().simple(),
        ))
        .unwrap();
        assert!(!test_support::catalog_cached(&db).unwrap());
        db
    }
}

fn one(path: &Path, source: Source, cursor: &Cursor) -> Result<PageRead> {
    read_round(path, "runtime", 1, |reader| reader.page(source, cursor))?
}

#[test]
fn eight_due_sources_share_one_catalog_and_match_legacy_pages() {
    use sha2::Digest as _;
    let fixture = Fixture::new();
    fixture.queued("read-round-pending", 40);
    fixture.completed("read-round-old", 42, 1.0);
    fixture.completed("read-round-later", 42, 2.0);
    fixture.completed("read-round-free", 44, 3.0);
    fixture.completed(&"z".repeat(super::super::MAX_METADATA_BYTES + 1), 43, 1.0);
    let key = serde_json::to_string(&(42, "completion/v1", "read-round-old", 0)).unwrap();
    crate::delivery_receipt::begin(
        &fixture.path,
        &key,
        &hex::encode(sha2::Sha256::digest(b"body")),
    )
    .unwrap();
    let observations = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&observations);
    let guard = HookGuard::install(move |point, _| {
        observed.borrow_mut().push(point);
        Ok(())
    });
    let expected = Source::ALL.map(|source| {
        let mut cursor = Cursor::default();
        let page = super::super::page(&fixture.path, source, &mut cursor, "runtime", 1).unwrap();
        (cursor, page)
    });
    assert_eq!(
        observations
            .borrow()
            .iter()
            .filter(|p| **p == Boundary::Opened)
            .count(),
        8
    );
    assert_eq!(
        observations
            .borrow()
            .iter()
            .filter(|p| **p == Boundary::Catalog)
            .count(),
        8
    );
    observations.borrow_mut().clear();
    let actual = read_round(&fixture.path, "runtime", 1, |reader| {
        Source::ALL.map(|source| reader.page(source, &Cursor::default()))
    })
    .unwrap();
    for (actual, (cursor, page)) in actual.into_iter().zip(expected) {
        let actual = actual.unwrap();
        assert_eq!(actual.cursor, cursor);
        assert_eq!(actual.page, page);
        if actual.page.held_receipt_heads != 0 {
            assert_eq!(actual.page.held_receipt_heads, 1);
            assert!(actual.page.oversized_identity);
            assert_eq!(actual.page.entries[0].id, "read-round-free");
        }
    }
    assert_eq!(
        observations
            .borrow()
            .iter()
            .filter(|p| **p == Boundary::Opened)
            .count(),
        1
    );
    assert_eq!(
        observations
            .borrow()
            .iter()
            .filter(|p| **p == Boundary::Catalog)
            .count(),
        1
    );
    drop(guard);
    let db = Connection::open(&fixture.path).unwrap();
    let receipt: (Option<String>, i64) = db
        .query_row(
            "SELECT message_id,retryable FROM codex_delivery_receipts WHERE receipt_key=?",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        receipt,
        (None, 0),
        "metadata cannot claim or retry the held POST"
    );
}

#[test]
fn empty_round_opens_nothing_and_each_source_is_bounded() {
    let fixture = Fixture::new();
    let absent = fixture.path.with_file_name("absent.sqlite");
    let points = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&points);
    let _guard = HookGuard::install(move |point, _| {
        observed.borrow_mut().push(point);
        Ok(())
    });
    assert_eq!(read_round(&absent, "runtime", 1, |_| 42).unwrap(), 42);
    assert!(points.borrow().is_empty());
    assert!(!absent.exists());
    read_round(&fixture.path, "runtime", 1, |reader| {
        for source in Source::ALL {
            reader.page(source, &Cursor::default()).unwrap();
        }
        assert!(reader.page(Source::Queue, &Cursor::default()).is_err());
    })
    .unwrap();
    assert_eq!(
        points
            .borrow()
            .iter()
            .filter(|p| **p == Boundary::Opened)
            .count(),
        1
    );
    assert_eq!(
        points
            .borrow()
            .iter()
            .filter(|p| matches!(p, Boundary::PageRead(_)))
            .count(),
        8
    );
}

#[test]
fn source_sql_failure_discards_its_cursor_but_keeps_independent_pages() {
    let fixture = Fixture::new();
    fixture.queued("read-round-pending", 40);
    fixture.completed("read-round-free", 44, 1.0);
    let cursor = Cursor::default();
    let _guard = HookGuard::install(|point, db| {
        if point == Boundary::PageRead(Source::Queue) {
            let _: i64 = db.query_row("SELECT missing_round_column", [], |row| row.get(0))?;
        }
        Ok(())
    });
    let (failed, independent) = read_round(&fixture.path, "runtime", 1, |reader| {
        (
            reader.page(Source::Queue, &cursor),
            reader.page(Source::Final, &Cursor::default()),
        )
    })
    .unwrap();
    assert!(failed.is_err());
    assert_eq!(
        cursor,
        Cursor::default(),
        "failed page cannot advance its input cursor"
    );
    assert_eq!(independent.unwrap().page.entries[0].id, "read-round-free");
}

#[test]
fn commit_failure_discards_all_produced_pages_and_does_not_seed_catalog() {
    let fixture = Fixture::new();
    fixture.queued("read-round-commit", 40);
    let db = fixture.uncached_catalog();
    let produced = Rc::new(RefCell::new(0));
    let observed = Rc::clone(&produced);
    let guard = HookGuard::install(move |point, connection| {
        if matches!(point, Boundary::PageRead(_)) {
            *observed.borrow_mut() += 1;
        }
        if point == Boundary::BeforeCommit {
            // The subsequent real COMMIT must fail: no transaction remains.
            connection.execute_batch("ROLLBACK")?;
        }
        Ok(())
    });
    let result = read_round(&fixture.path, "runtime", 1, |reader| {
        reader.page(Source::Queue, &Cursor::default()).unwrap()
    });
    assert!(result.is_err(), "uncommitted page must not be returned");
    assert_eq!(*produced.borrow(), 1);
    drop(guard);
    assert!(!test_support::catalog_cached(&db).unwrap());
    assert_eq!(
        one(&fixture.path, Source::Queue, &Cursor::default())
            .unwrap()
            .page
            .entries
            .len(),
        1
    );
}

#[test]
fn missing_or_invalid_schema_is_never_created_or_repaired() {
    let fixture = Fixture::new();
    let absent = fixture.path.with_file_name("absent.sqlite");
    assert!(one(&absent, Source::Queue, &Cursor::default()).is_err());
    assert!(!absent.exists());
    let db = Connection::open(&fixture.path).unwrap();
    db.execute_batch("DROP INDEX codex_mutation_prepared_target")
        .unwrap();
    assert!(one(&fixture.path, Source::Queue, &Cursor::default()).is_err());
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='codex_mutation_prepared_target')",
        [], |row| row.get(0),
    ).unwrap();
    assert!(!exists, "discovery must not silently rebuild a guard");
    db.pragma_update(None, "user_version", 999).unwrap();
    assert!(matches!(
        one(&fixture.path, Source::Queue, &Cursor::default()),
        Err(StoreError::UnsupportedVersion { found: 999, .. })
    ));
    assert_eq!(crate::schema::schema_version(&db).unwrap(), 999);
}

#[test]
fn concurrent_ddl_after_catalog_cannot_mix_metadata_snapshots() {
    let fixture = Fixture::new();
    fixture.completed("read-round-snapshot", 44, 1.0);
    let writer = fixture.uncached_catalog();
    let guard = HookGuard::install(move |point, _| {
        if point == Boundary::Catalog {
            writer.execute_batch(
                "UPDATE codex_delivery_outbox SET content='newer-body';
                 DROP INDEX codex_mutation_prepared_target;
                 PRAGMA user_version=999;",
            )?;
        }
        Ok(())
    });
    let result = one(&fixture.path, Source::Final, &Cursor::default()).unwrap();
    assert_eq!(
        result.page.entries[0].bytes, 4,
        "catalog and rows must share the old WAL snapshot"
    );
    drop(guard);
    let db = Connection::open(&fixture.path).unwrap();
    assert_eq!(crate::schema::schema_version(&db).unwrap(), 999);
    assert!(
        !test_support::catalog_cached(&db).unwrap(),
        "a raced invalid key must never be cached"
    );
    assert!(matches!(
        one(&fixture.path, Source::Final, &Cursor::default()),
        Err(StoreError::UnsupportedVersion { found: 999, .. })
    ));
}

#[test]
fn uncached_valid_schema_is_checked_read_only() {
    let fixture = Fixture::new();
    fixture.queued("read-round-readonly", 40);
    let db = fixture.uncached_catalog();
    let guard = HookGuard::install(|point, connection| {
        if matches!(point, Boundary::PageRead(_)) {
            assert!(
                connection
                    .execute("DELETE FROM codex_turn_queue", [])
                    .is_err()
            );
        }
        Ok(())
    });
    assert_eq!(
        one(&fixture.path, Source::Queue, &Cursor::default())
            .unwrap()
            .page
            .entries
            .len(),
        1
    );
    drop(guard);
    assert!(test_support::catalog_cached(&db).unwrap());
    let rows: i64 = db
        .query_row("SELECT count(*) FROM codex_turn_queue", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn round_high_water_excludes_new_tail_until_the_next_pass() {
    let fixture = Fixture::new();
    for index in 0..34 {
        fixture.queued(&format!("read-round-{index:03}"), 40);
    }
    let first = one(&fixture.path, Source::Queue, &Cursor::default()).unwrap();
    assert_eq!(first.page.entries.len(), 32);
    fixture.queued("zz-late", 40);
    let second = one(&fixture.path, Source::Queue, &first.cursor).unwrap();
    assert_eq!(second.page.entries.len(), 2);
    assert!(second.cursor.finished);
    assert_eq!(second.page.entries[1].id, "read-round-033");
    let mut cursor = Cursor::default();
    let mut ids = Vec::new();
    while !cursor.finished {
        let page = one(&fixture.path, Source::Queue, &cursor).unwrap();
        ids.extend(page.page.entries.into_iter().map(|entry| entry.id));
        cursor = page.cursor;
    }
    assert_eq!(ids.len(), 35);
    assert_eq!(ids.last().unwrap(), "zz-late");
}
