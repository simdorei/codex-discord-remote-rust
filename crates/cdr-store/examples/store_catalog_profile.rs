//! Compare fresh structural catalog reads in a temporary store only.
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::time::Instant;

const ROWS: &str = "SELECT v.user_version,s.type,s.name,s.tbl_name,s.sql
    FROM pragma_user_version AS v LEFT JOIN main.sqlite_schema AS s ON 1
    ORDER BY s.type,s.name";
const JSON: &str = "SELECT json_group_array(json_array(user_version,type,name,tbl_name,sql))
    FROM (SELECT v.user_version,s.type,s.name,s.tbl_name,s.sql
    FROM pragma_user_version AS v LEFT JOIN main.sqlite_schema AS s ON 1
    ORDER BY s.type,s.name)";

fn sample(mut operation: impl FnMut() -> usize) -> serde_json::Value {
    let mut timings = Vec::new();
    let mut size = 0;
    for _ in 0..100 {
        let started = Instant::now();
        size = operation();
        timings.push(started.elapsed().as_micros());
    }
    serde_json::json!({"us":timings,"size":size})
}

fn row_catalog(db: &Connection, hash: bool) -> usize {
    let mut statement = db.prepare(ROWS).unwrap();
    let mut rows = statement.query([]).unwrap();
    let mut digest = Sha256::new();
    let mut size = 0;
    while let Some(row) = rows.next().unwrap() {
        let version: i64 = row.get(0).unwrap();
        if hash {
            digest.update(version.to_le_bytes());
        }
        for column in 1_usize..5 {
            if let Some(value) = row.get::<_, Option<String>>(column).unwrap() {
                size += value.len();
                if hash {
                    digest.update([1]);
                    digest.update(value.len().to_le_bytes());
                    digest.update(value.as_bytes());
                }
            } else if hash {
                digest.update([0]);
            }
        }
    }
    if hash {
        std::hint::black_box(digest.finalize());
    }
    size
}

fn main() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    drop(cdr_store::schema::open_initialized(&path).unwrap());
    let initialized = sample(|| {
        drop(cdr_store::schema::open_initialized(&path).unwrap());
        0
    });
    let rows_hashed = sample(|| row_catalog(&Connection::open(&path).unwrap(), true));
    let rows_unhashed = sample(|| row_catalog(&Connection::open(&path).unwrap(), false));
    let json = sample(|| {
        let db = Connection::open(&path).unwrap();
        let value: String = db.query_row(JSON, [], |row| row.get(0)).unwrap();
        std::hint::black_box(value).len()
    });
    println!(
        "{}",
        serde_json::json!({
            "temporary_database_only":true,
            "initialized":initialized,"rows_hashed":rows_hashed,
            "rows_unhashed":rows_unhashed,"json":json
        })
    );
}
