use std::path::PathBuf;

use cdr_store::{
    ingress::stop::{StopScope, accept_unresolved, revision},
    queue::{self, NewQueueJob},
};
use rusqlite::{Connection, types::Value as SqlValue};
use serde_json::{Value, json};

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        cdr_store::schema::open_initialized(&db).unwrap();
        Self { _temp: temp, db }
    }

    fn original(&self) {
        queue::enqueue(
            &self.db,
            NewQueueJob {
                job_id: "original",
                target_thread_id: "recover-a",
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(701),
                app_server_generation: 7,
                prompt: "original input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        queue::begin_attempt(&self.db, "original", &["baseline".into()], 7).unwrap();
    }

    fn stop(&self) {
        accept_unresolved(&self.db, StopScope { target: "recover-a", channel: 99, owner: 20 },
            &json!({"target":"recover-a","route":"Explicit","command":{"Stop":{"reference":"recover-a"}}}),
            None, || Ok(())).unwrap().unwrap();
    }

    fn origin(&self, target: &str) -> Value {
        revision::capture(&self.db, Some(target)).unwrap()
    }

    fn valid(&self, target: &str, origin: &Value) -> bool {
        let mut db = Connection::open(&self.db).unwrap();
        let tx = db.transaction().unwrap();
        revision::validate_request_in(&tx, "thread/settings/update", Some(target), Some(origin))
            .is_ok()
    }

    fn table(&self, table: &str) -> Vec<Vec<SqlValue>> {
        let db = Connection::open(&self.db).unwrap();
        let mut query = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        query
            .query_map([], |row| {
                (0..row.as_ref().column_count())
                    .map(|i| row.get(i))
                    .collect()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn snapshot(&self) -> Vec<Vec<Vec<SqlValue>>> {
        [
            "codex_turn_queue",
            "codex_request_cancellations",
            "cdr_execution_holds",
            "cdr_stop_revision_receipts",
            "cdr_stop_clock",
            "cdr_stop_revisions",
        ]
        .iter()
        .map(|name| self.table(name))
        .collect()
    }

    fn latest_scope(&self) -> Value {
        let raw: String = Connection::open(&self.db)
            .unwrap()
            .query_row(
                "SELECT scope_json FROM cdr_stop_revision_receipts ORDER BY revision DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }
}

#[test]
fn empty_recovery_cancellation_revokes_the_previously_admitted_origin() {
    let f = Fixture::new();
    let before = f.origin("recover-a");
    let other = f.origin("independent-b");
    assert!(f.valid("recover-a", &before));
    let cancelled = queue::cancel_for_recovery(&f.db, "recover-a", 99, 20, 2.0).unwrap();
    assert!(cancelled.jobs.is_empty());
    assert!(
        !f.valid("recover-a", &before),
        "completed recovery cancellation must revoke the original ordinary RPC"
    );
    assert!(
        f.valid("independent-b", &other),
        "unrelated original admission remains valid"
    );
    let fresh = f.origin("recover-a");
    assert_eq!(fresh["stopRevision"], 1);
    assert!(f.valid("recover-a", &fresh));
    assert_eq!(
        f.latest_scope(),
        json!({
            "kind":"recovery-cancellation", "target":"recover-a", "channel":99,
            "owner":20, "jobs":[], "cancelled_at":2.0,
        })
    );
    assert!(f.table("codex_turn_queue").is_empty());
    assert!(f.table("cdr_stop_controls").is_empty());
}

#[test]
fn started_original_is_cancelled_without_replay_and_old_stop_history_is_retained() {
    let f = Fixture::new();
    f.original();
    f.stop();
    let old_receipts = f.table("cdr_stop_revision_receipts");
    let original = f.origin("recover-a");
    let claimed = queue::list(&f.db).unwrap().remove(0);
    let cancelled = queue::cancel_for_recovery(&f.db, "recover-a", 99, 20, 2.0).unwrap();
    assert_eq!(cancelled.jobs, ["original"]);
    assert_eq!(cancelled.started_or_uncertain, 1);
    assert!(!f.valid("recover-a", &original));
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert!(
        cdr_store::execution_hold::reason(&f.db, "original")
            .unwrap()
            .is_some()
    );
    assert_eq!(f.table("codex_request_cancellations").len(), 1);
    let receipts = f.table("cdr_stop_revision_receipts");
    assert_eq!(receipts.len(), 2);
    assert_eq!(
        receipts[0], old_receipts[0],
        "prior immutable stop receipt retained"
    );
    assert_eq!(f.latest_scope()["kind"], "recovery-cancellation");
    assert_eq!(f.latest_scope()["jobs"], json!(["original"]));
    assert!(
        f.latest_scope().get("binding").is_none(),
        "never forge a Stop binding"
    );
    assert!(
        queue::mark_running_if_claimed(&f.db, &claimed, "late-original-turn")
            .unwrap()
            .is_none()
    );
    assert!(f.table("cdr_stop_controls").is_empty());
}

#[test]
fn failed_or_corrupted_revision_write_rolls_back_all_cancellation_custody() {
    for (table, timing, event, body) in [
        (
            "cdr_stop_revision_receipts",
            "BEFORE",
            "INSERT",
            "SELECT RAISE(ABORT,'injected receipt failure');",
        ),
        (
            "cdr_stop_revision_receipts",
            "BEFORE",
            "INSERT",
            "SELECT RAISE(IGNORE);",
        ),
        (
            "cdr_stop_revision_receipts",
            "AFTER",
            "INSERT",
            "DELETE FROM cdr_stop_revision_receipts WHERE operation_id=NEW.operation_id;",
        ),
        (
            "cdr_stop_revision_receipts",
            "AFTER",
            "INSERT",
            "UPDATE cdr_stop_revision_receipts SET scope_json='{}' WHERE operation_id=NEW.operation_id;",
        ),
        (
            "cdr_stop_revision_receipts",
            "AFTER",
            "INSERT",
            "UPDATE cdr_stop_clock SET revision=revision+1;",
        ),
        (
            "cdr_stop_revisions",
            "AFTER",
            "UPDATE",
            "UPDATE cdr_stop_revisions SET operation_id='replacement' WHERE target_thread_id=NEW.target_thread_id;",
        ),
    ] {
        let f = Fixture::new();
        f.original();
        f.stop();
        let before = f.snapshot();
        Connection::open(&f.db).unwrap().execute_batch(&format!(
            "CREATE TRIGGER corrupt_recovery_revision {timing} {event} ON {table} BEGIN {body} END;"
        )).unwrap();
        assert!(
            queue::cancel_for_recovery(&f.db, "recover-a", 99, 20, 2.0).is_err(),
            "{table} {timing} {event}: {body}"
        );
        assert_eq!(
            f.snapshot(),
            before,
            "whole prior rows, holds, receipts and clock roll back"
        );
    }
}

#[test]
fn rejected_actor_never_creates_a_new_revision_or_cancels_the_original() {
    let f = Fixture::new();
    f.original();
    let before = f.snapshot();
    assert!(queue::cancel_for_recovery(&f.db, "recover-a", 99, 21, 2.0).is_err());
    assert_eq!(f.snapshot(), before);
}

#[test]
fn distinct_empty_recovery_requests_advance_order_without_creating_work() {
    let f = Fixture::new();
    queue::cancel_for_recovery(&f.db, "recover-a", 99, 20, 2.0).unwrap();
    let first = f.origin("recover-a");
    queue::cancel_for_recovery(&f.db, "recover-a", 99, 20, 3.0).unwrap();
    assert!(!f.valid("recover-a", &first));
    assert_eq!(f.origin("recover-a")["stopRevision"], 2);
    assert_eq!(f.table("cdr_stop_revision_receipts").len(), 2);
    assert!(f.table("codex_turn_queue").is_empty());
    assert!(f.table("codex_request_cancellations").is_empty());
    assert!(f.table("cdr_stop_controls").is_empty());
}
