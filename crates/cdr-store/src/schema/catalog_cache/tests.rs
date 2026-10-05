use super::*;
use crate::{StoreError, delivery_receipt, schema::open_initialized};
use std::path::Path;

fn warm(path: &Path) {
    for _ in 0..3 {
        drop(open_initialized(path).unwrap());
    }
}

fn has_mutation_index(db: &Connection) -> bool {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='codex_mutation_prepared_target')",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn catalog_signature_includes_version_and_full_ddl_but_not_rows() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE marker(value INTEGER);")
        .unwrap();
    let original = signature(&db).unwrap();
    db.execute("INSERT INTO marker VALUES (7)", []).unwrap();
    assert_eq!(original, signature(&db).unwrap());
    db.execute_batch("ALTER TABLE marker ADD COLUMN other TEXT;")
        .unwrap();
    let changed = signature(&db).unwrap();
    assert_ne!(original, changed);
    db.pragma_update(None, "user_version", 8).unwrap();
    assert_ne!(changed, signature(&db).unwrap());
}

#[test]
fn dropped_guard_is_repaired_even_if_schema_version_is_reused() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    warm(&path);
    let db = Connection::open(&path).unwrap();
    let version: i64 = db
        .pragma_query_value(None, "schema_version", |r| r.get(0))
        .unwrap();
    db.execute_batch("DROP INDEX codex_mutation_prepared_target;")
        .unwrap();
    db.pragma_update(None, "schema_version", version).unwrap();
    drop(db);
    let reopened = open_initialized(&path).unwrap();
    assert!(has_mutation_index(&reopened));
}

#[test]
fn same_path_replacement_cannot_borrow_the_original_catalog() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("store.sqlite");
    let replacement = temp.path().join("replacement.sqlite");
    warm(&original);
    warm(&replacement);
    let db = Connection::open(&replacement).unwrap();
    db.execute_batch(
        "CREATE TABLE replacement_marker(value TEXT);
         INSERT INTO replacement_marker VALUES ('preserved');
         DROP INDEX codex_mutation_prepared_target;",
    )
    .unwrap();
    drop(db);
    std::fs::rename(&original, temp.path().join("original-retained.sqlite")).unwrap();
    std::fs::rename(&replacement, &original).unwrap();
    let reopened = open_initialized(&original).unwrap();
    assert!(has_mutation_index(&reopened));
    assert_eq!(
        reopened
            .query_row("SELECT value FROM replacement_marker", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "preserved",
    );
}

#[test]
fn cached_catalog_does_not_hide_an_unsupported_store_version() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    warm(&path);
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 999)
        .unwrap();
    assert!(matches!(
        open_initialized(&path),
        Err(StoreError::UnsupportedVersion { found: 999, .. }),
    ));
}

#[test]
fn same_catalog_never_caches_delivery_ownership_or_database_contents() {
    use delivery_receipt::ReceiptState;
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a.sqlite");
    let b = temp.path().join("b.sqlite");
    warm(&a);
    warm(&b);
    assert_eq!(
        delivery_receipt::begin(&a, "receipt", "hash").unwrap(),
        ReceiptState::New
    );
    assert_eq!(
        delivery_receipt::begin(&b, "receipt", "hash").unwrap(),
        ReceiptState::New
    );
    assert!(delivery_receipt::confirm(&a, "receipt", "123").unwrap());
    assert_eq!(
        delivery_receipt::begin(&a, "receipt", "hash").unwrap(),
        ReceiptState::Delivered("123".into()),
    );
    assert_eq!(
        delivery_receipt::begin(&b, "receipt", "hash").unwrap(),
        ReceiptState::Unknown
    );
    assert_eq!(
        delivery_receipt::begin(&a, "receipt", "changed").unwrap(),
        ReceiptState::ContentConflict
    );
}

#[test]
fn cache_is_bounded_and_remembering_a_signature_is_idempotent() {
    let mut cache = CatalogCache::default();
    for id in 0_u8..65 {
        cache.remember(format!("catalog-{id:03}").into_boxed_str());
    }
    assert_eq!(cache.entries.len(), 64);
    assert!(!cache.entries.contains(&"catalog-000".into()));
    assert!(cache.entries.contains(&"catalog-064".into()));
    cache.remember("catalog-064".into());
    assert_eq!(cache.entries.len(), 64);
}

#[test]
fn exact_encoding_preserves_quoted_names_and_null_sql() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE \"quoted\"\"name\"(id TEXT PRIMARY KEY, value TEXT DEFAULT 'null,[]');",
    )
    .unwrap();
    let encoded = signature(&db).unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&encoded).unwrap();
    let table = rows.iter().find(|row| row[1] == "table").unwrap();
    assert_eq!(table[2], "quoted\"name");
    assert_eq!(table[3], "quoted\"name");
    assert!(table[4].as_str().unwrap().contains("'null,[]'"));
    let index = rows.iter().find(|row| row[1] == "index").unwrap();
    assert!(index[4].is_null());
    assert!(rows.iter().all(|row| row[0] == 0));
}

#[test]
fn byte_budget_evicts_old_catalogs_and_does_not_retain_oversized_entries() {
    let mut cache = CatalogCache::default();
    let half = MAX_CATALOG_BYTES / 2;
    for value in ["a", "b", "c"] {
        cache.remember(value.repeat(half).into_boxed_str());
    }
    assert_eq!(cache.entries.len(), 2);
    assert_eq!(cache.bytes, MAX_CATALOG_BYTES);
    assert!(cache.entries.front().unwrap().starts_with('b'));
    cache.remember("x".repeat(MAX_CATALOG_BYTES + 1).into_boxed_str());
    assert_eq!(cache.entries.len(), 2);
    assert_eq!(cache.bytes, MAX_CATALOG_BYTES);
}
