//! Bounded catalog contention measurements; private temporary databases only.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use rusqlite::Connection;
use serde_json::{Value, json};

const ITERATIONS: usize = 32;
const CATALOG: &str = "SELECT json_group_array(json_array(user_version,type,name,tbl_name,sql))
    FROM (SELECT v.user_version,s.type,s.name,s.tbl_name,s.sql
    FROM pragma_user_version AS v LEFT JOIN main.sqlite_schema AS s ON 1
    ORDER BY s.type,s.name)";

#[derive(Clone, Copy)]
enum Mode {
    Fresh,
    ReusedDiagnosticOnly,
}

struct Measurement {
    open_us: u128,
    prepare_us: u128,
    step_us: u128,
    close_us: u128,
    bytes: usize,
}

fn measure(path: &Path, mode: Mode, ready: &Barrier) -> Vec<Measurement> {
    ready.wait();
    let reused = match mode {
        Mode::Fresh => None,
        Mode::ReusedDiagnosticOnly => Some(Connection::open(path).unwrap()),
    };
    let mut rows = Vec::with_capacity(ITERATIONS);
    let mut expected = None;
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let local = match mode {
            Mode::Fresh => Some(Connection::open(path).unwrap()),
            Mode::ReusedDiagnosticOnly => None,
        };
        let db = local.as_ref().or(reused.as_ref()).unwrap();
        let opened = Instant::now();
        let mut statement = db.prepare(CATALOG).unwrap();
        let prepared = Instant::now();
        let value: String = statement.query_row([], |row| row.get(0)).unwrap();
        let stepped = Instant::now();
        let bytes = value.len();
        if let Some(original) = &expected {
            assert_eq!(
                original, &value,
                "catalog changed during read-only measurement"
            );
        } else {
            expected = Some(value);
        }
        drop(statement);
        drop(local);
        let closed = Instant::now();
        rows.push(Measurement {
            open_us: opened.duration_since(start).as_micros(),
            prepare_us: prepared.duration_since(opened).as_micros(),
            step_us: stepped.duration_since(prepared).as_micros(),
            close_us: closed.duration_since(stepped).as_micros(),
            bytes,
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

fn run(paths: &[PathBuf], mode: Mode) -> Value {
    let ready = Arc::new(Barrier::new(paths.len() + 1));
    let handles: Vec<_> = paths
        .iter()
        .map(|path| {
            let path = path.clone();
            let ready = Arc::clone(&ready);
            std::thread::spawn(move || measure(&path, mode, &ready))
        })
        .collect();
    let start = Instant::now();
    ready.wait();
    let rows: Vec<_> = handles
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect();
    json!({
        "threads":paths.len(),
        "mode":match mode { Mode::Fresh=>"fresh", Mode::ReusedDiagnosticOnly=>"reused_diagnostic_only" },
        "wall_us":start.elapsed().as_micros(),
        "encoded_bytes":rows[0].bytes,
        "open":distribution(rows.iter().map(|r|r.open_us).collect()),
        "prepare":distribution(rows.iter().map(|r|r.prepare_us).collect()),
        "step":distribution(rows.iter().map(|r|r.step_us).collect()),
        "close":distribution(rows.iter().map(|r|r.close_us).collect()),
    })
}

fn main() {
    let temp = tempfile::tempdir().unwrap();
    let seed = temp.path().join("seed.sqlite");
    let db = cdr_store::schema::open_initialized(&seed).unwrap();
    let options = db
        .prepare("PRAGMA compile_options")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let sqlite_version: String = db
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
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
    let mut runs = Vec::new();
    let counts = if parallel == 1 {
        vec![1]
    } else {
        vec![1, parallel]
    };
    for count in counts {
        for mode in [Mode::Fresh, Mode::ReusedDiagnosticOnly] {
            runs.push(run(&paths[..count], mode));
        }
    }
    println!(
        "{}",
        json!({
            "temporary_databases_only":true,"iterations_per_thread":ITERATIONS,
            "available_parallelism":available,"sqlite_version":sqlite_version,
            "compile_options":options,"synchronous":synchronous,"journal_mode":journal,
            "reused_connection_is_measurement_only_not_a_production_change":true,
            "runs":runs,
        })
    );
}
