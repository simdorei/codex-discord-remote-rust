//! Short-lived two-read connections versus two fresh connections; private DBs only.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use rusqlite::Connection;
use serde_json::{Value, json};

const PAIRS: usize = 16;
const CATALOG: &str = "SELECT json_group_array(json_array(user_version,type,name,tbl_name,sql))
    FROM (SELECT v.user_version,s.type,s.name,s.tbl_name,s.sql
    FROM pragma_user_version AS v LEFT JOIN main.sqlite_schema AS s ON 1
    ORDER BY s.type,s.name)";

#[derive(Clone, Copy)]
enum Mode {
    FreshPair,
    ShortLivedPair,
}

struct Pair {
    open: u128,
    first_prepare: u128,
    second_prepare: u128,
    query: u128,
    close: u128,
    total: u128,
}

fn read_catalog(db: &Connection, expected: &str) -> (u128, u128) {
    let before = Instant::now();
    let mut statement = db.prepare(CATALOG).unwrap();
    let prepared = Instant::now();
    let actual: String = statement.query_row([], |row| row.get(0)).unwrap();
    assert_eq!(actual, expected, "exact catalog changed");
    let queried = Instant::now();
    drop(statement);
    (
        prepared.duration_since(before).as_micros(),
        queried.duration_since(prepared).as_micros(),
    )
}

fn measure(path: &Path, mode: Mode, ready: &Barrier, expected: &str) -> Vec<Pair> {
    ready.wait();
    let mut rows = Vec::with_capacity(PAIRS);
    for _ in 0..PAIRS {
        let before = Instant::now();
        let mut db = Connection::open(path).unwrap();
        let mut open_us = before.elapsed().as_micros();
        let (first_prepare_us, first_query_us) = read_catalog(&db, expected);
        let mut close_us = 0;
        if matches!(mode, Mode::FreshPair) {
            let closing = Instant::now();
            drop(db);
            close_us += closing.elapsed().as_micros();
            let opening = Instant::now();
            db = Connection::open(path).unwrap();
            open_us += opening.elapsed().as_micros();
        }
        let (second_prepare_us, second_query_us) = read_catalog(&db, expected);
        let closing = Instant::now();
        drop(db);
        close_us += closing.elapsed().as_micros();
        rows.push(Pair {
            open: open_us,
            first_prepare: first_prepare_us,
            second_prepare: second_prepare_us,
            query: first_query_us + second_query_us,
            close: close_us,
            total: before.elapsed().as_micros(),
        });
    }
    rows
}

fn distribution(mut values: Vec<u128>) -> Value {
    values.sort_unstable();
    json!({
        "count":values.len(), "sum_us":values.iter().sum::<u128>(),
        "median_us":values[values.len()/2],
        "p95_us":values[(values.len()*95/100).min(values.len()-1)],
        "max_us":values.last().unwrap(),
    })
}

fn run(paths: &[PathBuf], mode: Mode, expected: &Arc<String>) -> Value {
    let ready = Arc::new(Barrier::new(paths.len() + 1));
    let handles: Vec<_> = paths
        .iter()
        .map(|path| {
            let path = path.clone();
            let ready = Arc::clone(&ready);
            let expected = Arc::clone(expected);
            std::thread::spawn(move || measure(&path, mode, &ready, &expected))
        })
        .collect();
    let before = Instant::now();
    ready.wait();
    let rows: Vec<_> = handles
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect();
    json!({
        "threads":paths.len(),
        "mode":match mode { Mode::FreshPair=>"fresh_pair", Mode::ShortLivedPair=>"short_lived_pair" },
        "wall_us":before.elapsed().as_micros(),
        "open":distribution(rows.iter().map(|r|r.open).collect()),
        "first_prepare":distribution(rows.iter().map(|r|r.first_prepare).collect()),
        "second_prepare":distribution(rows.iter().map(|r|r.second_prepare).collect()),
        "query":distribution(rows.iter().map(|r|r.query).collect()),
        "close":distribution(rows.iter().map(|r|r.close).collect()),
        "pair_total":distribution(rows.iter().map(|r|r.total).collect()),
    })
}

fn main() {
    let temp = tempfile::tempdir().unwrap();
    let seed = temp.path().join("seed.sqlite");
    let db = cdr_store::schema::open_initialized(&seed).unwrap();
    let expected: String = db.query_row(CATALOG, [], |row| row.get(0)).unwrap();
    let sqlite_version: String = db
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .unwrap();
    let options = db
        .prepare("PRAGMA compile_options")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let synchronous: i64 = db
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .unwrap();
    let journal: String = db
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    drop(db);
    let available = std::thread::available_parallelism().unwrap().get();
    let parallel = available.min(32);
    let paths: Vec<_> = (0..parallel)
        .map(|i| {
            let path = temp.path().join(format!("read-{i}.sqlite"));
            std::fs::copy(&seed, &path).unwrap();
            path
        })
        .collect();
    let expected = Arc::new(expected);
    let counts = if parallel == 1 {
        vec![1]
    } else {
        vec![1, parallel]
    };
    let mut runs = Vec::new();
    for count in counts {
        // Fixed ABBA order, not a retry-until-success loop.
        for mode in [
            Mode::FreshPair,
            Mode::ShortLivedPair,
            Mode::ShortLivedPair,
            Mode::FreshPair,
        ] {
            runs.push(run(&paths[..count], mode, &expected));
        }
    }
    println!(
        "{}",
        json!({
            "temporary_databases_only":true,"pairs_per_thread":PAIRS,
            "queries_per_pair":2,"available_parallelism":available,
            "sqlite_version":sqlite_version,"compile_options":options,
            "synchronous":synchronous,"journal_mode":journal,
            "all_queries_check_exact_seed_catalog":true,
            "pair_total_includes_both_reads_connection_creation_and_destruction":true,
            "component_query_includes_equality_check":true,
            "statement_drop_included_in_pair_total_not_component_close":true,
            "runtime_change":false,"runs":runs,
        })
    );
}
