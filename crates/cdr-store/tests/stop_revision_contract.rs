use cdr_store::{
    ingress::stop::{StopScope, accept_nonrunning, revision},
    queue::{self, NewQueueJob},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let f = Self { _temp: temp, db };
        f.enqueue("original", "target-a", 101);
        f
    }

    fn enqueue(&self, id: &str, target: &str, event: i64) {
        queue::enqueue(
            &self.db,
            NewQueueJob {
                job_id: id,
                target_thread_id: target,
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(event),
                app_server_generation: 7,
                prompt: "original input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
    }

    fn stop(&self, target: &str) -> cdr_store::Result<()> {
        let receipt = accept_nonrunning(
            &self.db,
            StopScope {
                target,
                channel: 99,
                owner: 20,
            },
            &json!({"target":target,"route":"Explicit","command":{"Stop":{"reference":target}}}),
            None,
            || Ok(()),
        )?;
        assert!(receipt.is_some());
        Ok(())
    }

    fn capture(&self, target: &str) -> Value {
        revision::capture(&self.db, Some(target)).unwrap()
    }

    fn valid(&self, target: &str, origin: Option<&Value>) -> bool {
        revision::validate_in(&Connection::open(&self.db).unwrap(), Some(target), origin).is_ok()
    }
}

#[test]
fn stop_invalidates_only_older_originals_for_the_same_target_after_reopen() {
    let f = Fixture::new();
    let a = f.capture("target-a");
    let b = f.capture("target-b");
    assert_eq!(a, json!({"target":"target-a","stopRevision":0}));
    f.stop("target-a").unwrap();
    assert!(!f.valid("target-a", Some(&a)));
    assert!(
        !f.valid("target-a", None),
        "legacy origins cannot borrow the latest revision"
    );
    assert!(f.valid("target-b", Some(&b)));
    assert!(f.valid("target-b", None));
    let fresh = f.capture("target-a");
    assert_eq!(fresh["stopRevision"], 1);
    assert!(f.valid("target-a", Some(&fresh)));
    assert!(
        cdr_store::execution_hold::reason(&f.db, "original")
            .unwrap()
            .is_some()
    );
    f.stop("target-a").unwrap();
    assert!(!f.valid("target-a", Some(&fresh)));
    assert_eq!(f.capture("target-a")["stopRevision"], 2);
    let rows: i64 = Connection::open(&f.db)
        .unwrap()
        .query_row("SELECT count(*) FROM cdr_stop_revision_receipts", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 2);
}

#[test]
fn later_stop_on_b_does_not_revoke_fresh_a_or_borrow_b_identity() {
    let f = Fixture::new();
    f.enqueue("independent", "target-b", 102);
    f.stop("target-a").unwrap();
    let a = f.capture("target-a");
    let b = f.capture("target-b");
    f.stop("target-b").unwrap();
    assert!(f.valid("target-a", Some(&a)));
    assert!(!f.valid("target-b", Some(&b)));
    assert!(!f.valid("target-b", Some(&a)));
    let db = Connection::open(&f.db).unwrap();
    let scopes: Vec<Value> = db
        .prepare("SELECT scope_json FROM cdr_stop_revision_receipts ORDER BY revision")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|s| serde_json::from_str(&s.unwrap()).unwrap())
        .collect();
    assert_eq!(scopes[0]["jobs"], json!(["original"]));
    assert_eq!(scopes[1]["jobs"], json!(["independent"]));
    assert_eq!(scopes[0]["owner"], 20);
    assert_eq!(scopes[0]["channel"], 99);
}

#[test]
fn malformed_future_or_different_target_origins_fail_closed() {
    let f = Fixture::new();
    for origin in [
        Value::Null,
        json!({}),
        json!({"target":"target-a","stopRevision":-1}),
        json!({"target":"target-a","stopRevision":1}),
        json!({"target":"target-a","stopRevision":"0"}),
        json!({"target":"target-b","stopRevision":0}),
        json!({"target":null,"stopRevision":0}),
        json!({"target":"target-a","stopRevision":0,"extra":true}),
    ] {
        assert!(
            !f.valid("target-a", Some(&origin)),
            "accepted malformed origin {origin}"
        );
    }
}

#[test]
fn missing_or_inconsistent_revision_evidence_is_not_a_clean_snapshot() {
    for tamper in [
        "DELETE FROM cdr_stop_revisions",
        "UPDATE cdr_stop_revisions SET operation_id='different'",
        "DELETE FROM cdr_stop_revision_receipts",
        "UPDATE cdr_stop_clock SET revision=0",
        "DELETE FROM cdr_stop_clock",
    ] {
        let f = Fixture::new();
        f.stop("target-a").unwrap();
        let original = f.capture("target-a");
        Connection::open(&f.db)
            .unwrap()
            .execute_batch(tamper)
            .unwrap();
        assert!(
            revision::capture(&f.db, Some("target-a")).is_err(),
            "{tamper}"
        );
        assert!(!f.valid("target-a", Some(&original)), "{tamper}");
    }
}

#[test]
fn failed_ignored_or_rewritten_revision_write_rolls_back_all_stop_holds() {
    for body in [
        "SELECT RAISE(ABORT,'injected revision failure');",
        "SELECT RAISE(IGNORE);",
        "UPDATE cdr_stop_clock SET revision=0;",
    ] {
        let f = Fixture::new();
        let before = queue::list_filtered(&f.db, None, None).unwrap();
        let db = Connection::open(&f.db).unwrap();
        db.execute_batch(&format!(
            "CREATE TRIGGER break_revision BEFORE INSERT ON cdr_stop_revision_receipts
             BEGIN {body} END;"
        ))
        .unwrap();
        assert!(f.stop("target-a").is_err(), "{body}");
        assert_eq!(queue::list_filtered(&f.db, None, None).unwrap(), before);
        assert!(
            cdr_store::execution_hold::reason(&f.db, "original")
                .unwrap()
                .is_none()
        );
        assert_eq!(f.capture("target-a")["stopRevision"], 0);
        let history: i64 = db
            .query_row("SELECT count(*) FROM cdr_stop_revision_receipts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(history, 0);
    }
}

#[test]
fn capture_never_creates_a_missing_database_or_silently_migrates_an_old_one() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("absent.sqlite");
    assert!(revision::capture(&missing, Some("target-a")).is_err());
    assert!(!missing.exists());
    let f = Fixture::new();
    Connection::open(&f.db).unwrap().execute_batch(
        "DROP TABLE cdr_stop_revisions; DROP TABLE cdr_stop_revision_receipts; DROP TABLE cdr_stop_clock;",
    ).unwrap();
    assert!(revision::capture(&f.db, Some("target-a")).is_err());
    f.enqueue("after-migration", "target-b", 102);
    assert_eq!(f.capture("target-a")["stopRevision"], 0);
}
