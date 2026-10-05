use cdr_store::{
    ingress::{
        self, IngressKind, NewIngress, StoredIngress,
        stop::{StopScope, accept_nonrunning, accept_unresolved, control, revision},
    },
    queue::{self, NewQueueJob},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    binding: Value,
    stop: StoredIngress,
}
fn scope() -> StopScope<'static> {
    StopScope {
        target: "target",
        channel: 99,
        owner: 20,
    }
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let binding =
            json!({"target":"target","route":"Explicit","command":{"Stop":{"reference":"target"}}});
        ingress::admit(&db,&NewIngress {
            ingress_id:"message:500".into(),kind:IngressKind::Message,event_id:Some(500),application_id:None,
            channel_id:99,owner_user_id:20,source_message_id:Some(500),
            payload:json!({"version":1,"content":"!stop target","plan":{"Execute":binding["command"]},
                "lifecycle_binding":binding}),target_thread_id:Some("target".into()),canonical_owner:None,now:1.0,
        }).unwrap();
        ingress::begin_execution(&db, "message:500", "processing", Some("target"), 2.0).unwrap();
        let stop = ingress::get(&db, "message:500").unwrap().unwrap();
        Self {
            _temp: temp,
            db,
            binding,
            stop,
        }
    }
    fn accept(&self) -> cdr_store::Result<Option<ingress::stop::StopReceipt>> {
        accept_unresolved(
            &self.db,
            scope(),
            &self.binding,
            Some(&self.stop),
            || Ok(()),
        )
    }
    fn job(&self, state: &str, channel: i64, owner: i64) {
        queue::enqueue(
            &self.db,
            NewQueueJob {
                job_id: "original",
                target_thread_id: "target",
                channel_id: channel,
                owner_user_id: Some(owner),
                discord_message_id: Some(701),
                app_server_generation: 7,
                prompt: "original input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        if matches!(state, "starting" | "running" | "quarantined") {
            queue::begin_attempt(&self.db, "original", &[], 7).unwrap();
        }
        if state == "running" {
            queue::mark_running(&self.db, "original", "owned-turn", 7).unwrap();
        }
        if state == "quarantined" {
            Connection::open(&self.db)
                .unwrap()
                .execute(
                    "UPDATE codex_turn_queue SET state='running', turn_id='cdr-quarantined:fixture', last_error='[cdr-rust:app-server-fork-quarantine:v1] fixture' WHERE job_id='original'",
                    [],
                )
                .unwrap();
        }
    }
    fn current_stop(&self) -> StoredIngress {
        ingress::get(&self.db, "message:500").unwrap().unwrap()
    }
    fn revision(&self) -> Value {
        revision::capture(&self.db, Some("target")).unwrap()
    }
    fn empty_holds(&self) -> bool {
        Connection::open(&self.db)
            .unwrap()
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM cdr_execution_holds)",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
}

#[test]
fn empty_scope_records_intent_not_execution_end_and_preserves_receipt() {
    let f = Fixture::new();
    let old = f.revision();
    let receipt = f.accept().unwrap().unwrap();
    assert!(receipt.jobs.is_empty() && receipt.ingresses.is_empty());
    let saved = f.current_stop();
    assert_eq!(saved.phase, "stop_accepted");
    assert_eq!(
        saved.outcome.as_ref().unwrap()["execution_end_confirmed"],
        false
    );
    assert_eq!(f.revision()["stopRevision"], 1);
    assert!(
        revision::validate_in(
            &Connection::open(&f.db).unwrap(),
            Some("target"),
            Some(&old)
        )
        .is_err()
    );
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    assert!(f.empty_holds(), "no invented job or interrupt owner");
    ingress::record_result(&f.db, "message:500", &json!({"response":"accepted"}), 4.0).unwrap();
    assert_eq!(
        f.current_stop().outcome.unwrap()["stop_receipt"],
        saved.outcome.unwrap()
    );
}

#[test]
fn unresolved_queue_states_keep_original_identity_and_preexisting_hold() {
    for state in ["pending", "starting", "running", "quarantined"] {
        let f = Fixture::new();
        f.job(state, 99, 20);
        let before = queue::list(&f.db).unwrap();
        let db = Connection::open(&f.db).unwrap();
        db.execute(
            "INSERT INTO cdr_execution_holds
             (job_id,target_thread_id,reason,evidence_json,created_at)
             VALUES ('original','target','prior independent hold',?,1.0)",
            [r#"{"keep":true}"#],
        )
        .unwrap();
        let receipt = f.accept().unwrap().unwrap();
        assert_eq!(receipt.jobs, ["original"]);
        assert_eq!(queue::list(&f.db).unwrap(), before, "{state}");
        assert_eq!(
            cdr_store::execution_hold::reason(&f.db, "original")
                .unwrap()
                .as_deref(),
            Some("prior independent hold")
        );
        let evidence: String = db
            .query_row(
                "SELECT evidence_json FROM cdr_execution_holds WHERE job_id='original'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evidence, "{\"keep\":true}");
        assert!(
            control::pending_after(&f.db, 0).unwrap().is_empty(),
            "unverified owner is not an interrupt claim"
        );
    }
}

#[test]
fn foreign_scope_rolls_back_without_a_receipt_or_revision() {
    for (channel, owner) in [(98, 20), (99, 21)] {
        let f = Fixture::new();
        f.job("running", channel, owner);
        let before = queue::list(&f.db).unwrap();
        assert!(f.accept().is_err());
        assert_eq!(queue::list(&f.db).unwrap(), before);
        assert_eq!(f.current_stop(), f.stop);
        assert_eq!(f.revision()["stopRevision"], 0);
        assert!(f.empty_holds());
    }
}

#[test]
fn failed_ignored_or_rewritten_revision_rolls_back_even_empty_acceptance() {
    for body in [
        "SELECT RAISE(ABORT,'injected failure');",
        "SELECT RAISE(IGNORE);",
        "UPDATE cdr_stop_clock SET revision=0;",
    ] {
        let f = Fixture::new();
        Connection::open(&f.db).unwrap().execute_batch(&format!(
            "CREATE TRIGGER break_receipt BEFORE INSERT ON cdr_stop_revision_receipts BEGIN {body} END;"
        )).unwrap();
        assert!(f.accept().is_err());
        assert_eq!(f.current_stop(), f.stop);
        assert_eq!(f.revision()["stopRevision"], 0);
        assert!(f.empty_holds());
    }
}

#[test]
fn failed_stop_ingress_cas_does_not_leave_queue_holds() {
    let f = Fixture::new();
    f.job("running", 99, 20);
    let before = queue::list(&f.db).unwrap();
    Connection::open(&f.db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER ignore_stop BEFORE UPDATE OF phase ON discord_ingress_journal
         WHEN NEW.phase='stop_accepted' BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    assert!(f.accept().is_err());
    assert_eq!(f.current_stop(), f.stop);
    assert_eq!(queue::list(&f.db).unwrap(), before);
    assert_eq!(f.revision()["stopRevision"], 0);
    assert!(f.empty_holds());
}

#[test]
fn duplicate_original_cannot_expand_scope_or_refresh_revision() {
    let f = Fixture::new();
    f.accept().unwrap().unwrap();
    let first = f.current_stop();
    ingress::admit(
        &f.db,
        &NewIngress {
            ingress_id: "message:702".into(),
            kind: IngressKind::Message,
            event_id: Some(702),
            application_id: None,
            channel_id: 99,
            owner_user_id: 20,
            source_message_id: Some(702),
            payload: json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"new request"}}}}),
            target_thread_id: Some("target".into()),
            canonical_owner: None,
            now: 4.0,
        },
    )
    .unwrap();
    let later = ingress::get(&f.db, "message:702").unwrap().unwrap();
    assert!(f.accept().is_err());
    assert_eq!(f.current_stop(), first);
    assert_eq!(ingress::get(&f.db, "message:702").unwrap().unwrap(), later);
    assert_eq!(f.revision()["stopRevision"], 1);
}

#[test]
fn original_nonrunning_api_keeps_its_narrow_contract() {
    let f = Fixture::new();
    assert!(
        accept_nonrunning(&f.db, scope(), &f.binding, Some(&f.stop), || Ok(()))
            .unwrap()
            .is_none()
    );
    f.job("running", 99, 20);
    assert!(
        accept_nonrunning(&f.db, scope(), &f.binding, Some(&f.stop), || Ok(()))
            .unwrap()
            .is_none()
    );
    assert_eq!(f.current_stop(), f.stop);
    assert_eq!(f.revision()["stopRevision"], 0);
    assert!(f.empty_holds());
}

#[test]
fn missing_database_is_not_a_successful_stop_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing.sqlite");
    let binding =
        json!({"target":"target","route":"Explicit","command":{"Stop":{"reference":"target"}}});
    assert!(
        accept_unresolved(&missing, scope(), &binding, None, || Ok(()))
            .unwrap()
            .is_none()
    );
    assert!(!missing.exists());
}
