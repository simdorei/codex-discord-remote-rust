use super::*;
use crate as store;
use crate::schema::checked_read::test_support::{Boundary, HookGuard};
use std::{cell::RefCell, rc::Rc};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    db: Connection,
}

fn evidence(id: &str, target: &str, valid: bool) -> (Value, Value) {
    let job = format!("lost-{id}");
    let body = json!({"index":0,"source_text":"","title":"pick","options":["one"]});
    let claim = json!({"id":if valid {id.to_owned()} else {format!("different-{id}")},
        "runtime_id":"runtime","generation":1,"thread_id":target,"turn_id":"turn",
        "item_id":"item","origin_job_id":job,"channel_id":20,"owner_user_id":1,
        "body":body.to_string(),"message_id":"message","chosen":0,"dispatch_mode":"steer"});
    let seal = json!({"identity":{"question":["runtime",target,"turn","item",job],
        "generation":1,"channel":20,"actor":1,"message":"message","chosen":0,
        "body":body,"reply_job_id":null,"job":{"job_id":job,"target_thread_id":target,
        "channel_id":20,"owner_user_id":1,"turn_id":"turn"}}});
    (claim, seal)
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preflight.sqlite");
        let db = store::schema::open_initialized(&path).unwrap();
        Self {
            _directory: directory,
            path,
            db,
        }
    }

    fn seed(&self, id: &str, target: &str, valid: bool) {
        let (claim, seal) = evidence(id, target, valid);
        self.db
            .execute(
                "INSERT INTO cdr_async_execution_obligations
            (question_id,thread_id,origin_job_id,turn_id,channel_id,format_version,revision,
             answer_state,execution_state,admission_state,policy,original_seal,claim_json,
             owner_json,original_error,created_at,updated_at)
            VALUES(?1,?2,?3,'turn',20,1,0,'unresolved','unresolved','held','ordinary',
                ?4,?5,NULL,'original uncertainty',1,1)",
                params![
                    id,
                    target,
                    format!("lost-{id}"),
                    seal.to_string(),
                    claim.to_string()
                ],
            )
            .unwrap();
    }

    // Simulate an out-of-band repair in a disposable fixture only. Restore the
    // exact immutable trigger in the same transaction; no production repair API.
    fn rewrite(&mut self, sql: &str, values: &[&dyn rusqlite::ToSql]) {
        let trigger: String = self
            .db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='cdr_async_obligation_claim_immutable'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let tx = self.db.transaction().unwrap();
        tx.execute_batch("DROP TRIGGER cdr_async_obligation_claim_immutable")
            .unwrap();
        tx.execute(sql, values).unwrap();
        tx.execute_batch(&trigger).unwrap();
        tx.commit().unwrap();
    }

    fn repair(&mut self, id: &str, target: &str) {
        let (claim, seal) = evidence(id, target, true);
        self.rewrite("UPDATE cdr_async_execution_obligations SET claim_json=?1,original_seal=?2 WHERE question_id=?3",
            &[&claim.to_string(), &seal.to_string(), &id]);
    }
}

fn inspect(fixture: &Fixture) -> Result<(PageRead, Vec<usize>)> {
    read_round(&fixture.path, "runtime", 1, |reader| {
        let page = reader.page(Source::AsyncOrphan, &Cursor::default())?;
        let negative = reader.unprovable_orphans(&page.page.entries)?;
        Ok((page, negative))
    })?
}

#[test]
fn negative_page_shares_one_checked_snapshot_and_keeps_inventory() {
    let fixture = Fixture::new();
    for index in 0..32 {
        fixture.seed(&format!("q-{index:03}"), &format!("t-{index:03}"), false);
    }
    let points = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&points);
    let _guard = HookGuard::install(move |point, _| {
        observed.borrow_mut().push(point);
        Ok(())
    });
    let (page, negative) = inspect(&fixture).unwrap();
    assert_eq!(
        page.page.entries.len(),
        32,
        "preflight must not filter inventory"
    );
    assert_eq!(negative, (0..32).collect::<Vec<_>>());
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
            .filter(|p| **p == Boundary::Catalog)
            .count(),
        1
    );
    let materialized: usize = points
        .borrow()
        .iter()
        .filter_map(|p| match p {
            Boundary::OrphanMaterialized(rows, _) => Some(*rows),
            _ => None,
        })
        .sum();
    assert_eq!(materialized, 32);
    let retained: i64 = fixture
        .db
        .query_row(
            "SELECT count(*) FROM cdr_async_execution_obligations
        WHERE execution_state='unresolved' AND admission_state='held' AND revision=0
        AND original_error='original uncertainty'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 32);
}

#[test]
fn valid_saved_seal_needs_no_live_question_and_ignores_settled_old_evidence() {
    let fixture = Fixture::new();
    fixture.seed("current", "target", true);
    fixture.seed("old", "target", false);
    let proof = json!({"version":1,"owner_verified":true,"revision":0}).to_string();
    fixture.db.execute("UPDATE cdr_async_execution_obligations SET revision=1,
        execution_state='terminal',admission_state='settled',terminal_proof_json=? WHERE question_id='old'", [&proof]).unwrap();
    fixture
        .db
        .execute(
            "INSERT INTO cdr_async_terminal_settlements(question_id,revision,proof_json)
        VALUES('old',1,?)",
            [&proof],
        )
        .unwrap();
    let (_, negative) = inspect(&fixture).unwrap();
    assert!(
        negative.is_empty(),
        "live question absence is not a new rejection rule"
    );
    let questions: i64 = fixture
        .db
        .query_row("SELECT count(*) FROM cdr_async_questions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(questions, 0);
    assert!(
        store::async_resolution::capture_history_snapshot(&fixture.path, "target")
            .unwrap()
            .is_some()
    );
}

#[test]
fn row_budget_is_shared_across_targets_and_overflow_has_no_payload() {
    let fixture = Fixture::new();
    for target in ["a", "b"] {
        for index in 0..65 {
            fixture.seed(&format!("{target}-{index:03}"), target, false);
        }
    }
    let rows = Rc::new(RefCell::new(0));
    let observed = Rc::clone(&rows);
    let _guard = HookGuard::install(move |point, _| {
        if let Boundary::OrphanMaterialized(count, _) = point {
            *observed.borrow_mut() += count;
        }
        Ok(())
    });
    let (_, negative) = inspect(&fixture).unwrap();
    assert_eq!(
        negative,
        vec![0],
        "second target exceeds the shared 128-row budget"
    );
    assert_eq!(
        *rows.borrow(),
        65,
        "overflow row may only be a scalar probe"
    );
}

#[test]
fn byte_budget_is_shared_and_large_identifiers_are_not_materialized() {
    let mut fixture = Fixture::new();
    for id in ["a", "b", "c"] {
        fixture.seed(id, id, false);
    }
    fixture
        .db
        .execute(
            "UPDATE cdr_async_execution_obligations SET original_error=?",
            ["x".repeat(750_000)],
        )
        .unwrap();
    let (_, negative) = inspect(&fixture).unwrap();
    assert_eq!(
        negative,
        vec![0, 1],
        "third payload must remain undetermined"
    );
    fixture
        .db
        .execute(
            "UPDATE cdr_async_execution_obligations SET original_error='small'",
            [],
        )
        .unwrap();
    fixture.rewrite(
        "UPDATE cdr_async_execution_obligations SET origin_job_id=? WHERE question_id='a'",
        &[&"x".repeat(3 * 1024 * 1024)],
    );
    let (_, negative) = inspect(&fixture).unwrap();
    assert_eq!(
        negative,
        vec![1, 2],
        "an oversized identity is not a validator refusal"
    );
}

#[test]
fn preflight_sql_error_is_not_negative_and_independent_source_can_progress() {
    let fixture = Fixture::new();
    fixture.seed("q", "target", false);
    let _guard = HookGuard::install(|point, db| {
        if point == Boundary::OrphanPreflight {
            let _: i64 = db.query_row("SELECT missing_preflight_column", [], |row| row.get(0))?;
        }
        Ok(())
    });
    let (negative, other) = read_round(&fixture.path, "runtime", 1, |reader| {
        let page = reader
            .page(Source::AsyncOrphan, &Cursor::default())
            .unwrap();
        (
            reader.unprovable_orphans(&page.page.entries),
            reader.page(Source::Observed, &Cursor::default()),
        )
    })
    .unwrap();
    assert!(matches!(negative, Err(StoreError::Database(_))));
    assert!(other.is_ok());
}

#[test]
fn finish_failure_never_publishes_produced_negatives() {
    let fixture = Fixture::new();
    fixture.seed("q", "target", false);
    let produced = Rc::new(RefCell::new(false));
    let observed = Rc::clone(&produced);
    let _guard = HookGuard::install(move |point, db| {
        if matches!(point, Boundary::OrphanMaterialized(..)) {
            *observed.borrow_mut() = true;
        }
        if point == Boundary::BeforeCommit {
            db.execute_batch("ROLLBACK")?;
        }
        Ok(())
    });
    assert!(inspect(&fixture).is_err());
    assert!(*produced.borrow());
}

#[test]
fn repaired_evidence_is_read_again_without_a_negative_cache() {
    let mut fixture = Fixture::new();
    fixture.seed("q", "target", false);
    let (_, old) = inspect(&fixture).unwrap();
    assert_eq!(old, vec![0]);
    fixture.repair("q", "target");
    let (_, current) = inspect(&fixture).unwrap();
    assert!(current.is_empty());
    assert!(
        store::async_resolution::capture_history_snapshot(&fixture.path, "target")
            .unwrap()
            .is_some()
    );
}
