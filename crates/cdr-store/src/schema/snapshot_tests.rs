//! Deterministic interleavings against a separate WAL writer; temporary DBs only.
use std::cell::RefCell;
use std::sync::mpsc;
use std::time::Duration;

use rusqlite::Connection;

use super::{LATEST_STORE_SCHEMA_VERSION, StoreError, open_initialized};

#[derive(Clone, Copy)]
pub(super) enum Boundary {
    InitialCatalog,
    Initialized,
    FinalCatalog,
}

type Hook = Box<dyn FnMut(Boundary)>;
thread_local! {
    static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

pub(super) fn boundary(stage: Boundary) {
    HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(stage);
        }
    });
}

struct HookGuard;

impl HookGuard {
    fn install(hook: Hook) -> Self {
        HOOK.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(hook);
        });
        Self
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        HOOK.with(|slot| {
            let _ = slot.borrow_mut().take();
        });
    }
}

#[derive(Clone, Copy)]
enum Change {
    Index,
    Version,
}

impl Change {
    fn apply(self, db: &Connection, good: bool, index_sql: &str) {
        match self {
            Self::Index => {
                db.execute_batch(if good {
                    index_sql
                } else {
                    "DROP INDEX IF EXISTS codex_mutation_prepared_target;"
                })
                .unwrap();
            }
            Self::Version => db
                .pragma_update(
                    None,
                    "user_version",
                    if good {
                        LATEST_STORE_SCHEMA_VERSION
                    } else {
                        999
                    },
                )
                .unwrap(),
        }
    }
}

fn raced_store(change: Change) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    let db = open_initialized(&path).unwrap();
    let mode: String = db
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    db.execute_batch(&format!(
        "CREATE TABLE cache_seed_marker_{} (value TEXT);",
        uuid::Uuid::new_v4().simple(),
    ))
    .unwrap();
    let index_sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='codex_mutation_prepared_target'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    change.apply(&db, false, &index_sql);
    drop(db);
    let (commands, commands_rx) = mpsc::sync_channel(0);
    let (acknowledged, acknowledgements) = mpsc::sync_channel(0);
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        let db = Connection::open(writer_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        while let Ok(good) = commands_rx.recv() {
            change.apply(&db, good, &index_sql);
            if acknowledged.send(()).is_err() {
                break;
            }
        }
    });
    let guard = HookGuard::install(Box::new(move |stage| {
        // Bad key -> good initialize -> bad final key -> good live catalog.
        commands
            .send(!matches!(stage, Boundary::Initialized))
            .unwrap();
        acknowledgements
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
    }));
    let first = open_initialized(&path);
    drop(guard);
    writer.join().unwrap();
    drop(first.unwrap());
    let db = Connection::open(&path).unwrap();
    change.apply(&db, false, "");
    drop(db);
    temp
}

#[test]
fn concurrent_ddl_never_seeds_unvalidated_catalog() {
    let temp = raced_store(Change::Index);
    let db = open_initialized(&temp.path().join("store.sqlite")).unwrap();
    let repaired: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='codex_mutation_prepared_target')",
        [],
        |r| r.get(0),
    ).unwrap();
    assert!(
        repaired,
        "cache hit bypassed required index repair after concurrent DDL"
    );
}

#[test]
fn concurrent_version_never_seeds_unsupported_catalog() {
    let temp = raced_store(Change::Version);
    assert!(
        matches!(
            open_initialized(&temp.path().join("store.sqlite")),
            Err(StoreError::UnsupportedVersion { found: 999, .. }),
        ),
        "cache hit bypassed unsupported-version rejection after concurrent change",
    );
}
