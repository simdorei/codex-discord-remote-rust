use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cdr_store::schema::{assert_integrity, open_initialized};
use rusqlite::{Connection, MAIN_DB, OpenFlags};

fn object_names(conn: &Connection, kind: &str) -> BTreeSet<String> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = ? AND name NOT LIKE 'sqlite_%'")
        .expect("prepare sqlite_schema query");
    statement
        .query_map([kind], |row| row.get(0))
        .expect("query sqlite_schema")
        .collect::<rusqlite::Result<_>>()
        .expect("collect sqlite_schema names")
}

fn schema_object_keys(conn: &Connection) -> BTreeSet<(String, String)> {
    let mut statement = conn
        .prepare(
            "SELECT type, name FROM sqlite_schema \
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .expect("prepare full schema query");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query full schema")
        .collect::<rusqlite::Result<_>>()
        .expect("collect full schema")
}

fn frozen_v2_object_keys() -> BTreeSet<(String, String)> {
    let fixture = Connection::open_in_memory().expect("open frozen v2 fixture");
    fixture
        .execute_batch(include_str!(
            "../../../fixtures/parity/discord_mirror_schema_v2.sql"
        ))
        .expect("load frozen v2 fixture");
    schema_object_keys(&fixture)
}

fn table_counts(conn: &Connection) -> BTreeSet<(String, i64)> {
    object_names(conn, "table")
        .into_iter()
        .map(|name| {
            let quoted = name.replace('"', "\"\"");
            let count = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{quoted}\""), [], |row| {
                    row.get(0)
                })
                .expect("count table rows");
            (name, count)
        })
        .collect()
}

#[test]
#[ignore = "requires the repository live discord_mirror.sqlite"]
fn live_store_copy_opens_without_schema_or_row_loss() {
    let live_path = std::env::var_os("CDR_LIVE_STORE_PATH").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("discord_mirror.sqlite")
        },
        PathBuf::from,
    );
    assert!(live_path.is_file(), "live Discord mirror store is missing");
    let source = Connection::open_with_flags(&live_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open live store read-only");
    let temp = tempfile::tempdir().expect("create temporary directory");
    let copy_path = temp.path().join("live-copy.sqlite");
    source
        .backup(MAIN_DB, &copy_path, None)
        .expect("online backup live store to disposable copy");
    drop(source);

    let (before_schema, before_counts) = {
        let copy = Connection::open(&copy_path).expect("open raw live copy");
        (schema_object_keys(&copy), table_counts(&copy))
    };
    assert!(
        frozen_v2_object_keys().is_subset(&before_schema),
        "live store is missing an object from the frozen Python v2 schema"
    );
    let copy = open_initialized(&copy_path).expect("open copied live store through Rust");
    assert_integrity(&copy).expect("copied live store integrity");
    let after_schema = schema_object_keys(&copy);
    let after_counts = table_counts(&copy);
    assert!(
        before_schema.is_subset(&after_schema),
        "Rust initialization removed a pre-existing schema object"
    );
    assert!(
        before_counts.is_subset(&after_counts),
        "Rust initialization changed a pre-existing table row count"
    );
    assert!(
        object_names(&copy, "table").contains("codex_delivery_outbox"),
        "Rust delivery outbox extension is missing"
    );
}
