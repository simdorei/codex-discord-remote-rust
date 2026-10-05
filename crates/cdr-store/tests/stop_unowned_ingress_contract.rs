use std::path::{Path, PathBuf};

use cdr_store::{
    ingress::{
        self, IngressKind, NewIngress, StoredIngress,
        stop::{StopScope, accept_nonrunning, control},
    },
    prompt_intake::{self as intake, NewPromptIntake},
    queue::{self, NewQueueJob},
};
use rusqlite::Connection;
use serde_json::{Value, json};

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

fn job(id: &str, event: i64) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id: id,
        target_thread_id: "target",
        channel_id: 99,
        owner_user_id: Some(20),
        discord_message_id: Some(event),
        app_server_generation: 7,
        prompt: "original input",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}

fn prompt(db: &Path, event: i64, channel: i64, owner: i64) -> StoredIngress {
    let key = format!("message:{event}");
    ingress::admit(
        db,
        &NewIngress {
            ingress_id: key.clone(),
            kind: IngressKind::Message,
            event_id: Some(event),
            application_id: None,
            channel_id: channel,
            owner_user_id: owner,
            source_message_id: Some(event),
            payload: json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"original input"}}},
            "attachments":[{"artifact_status":"still preparing"}]}),
            target_thread_id: Some("target".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    assert!(ingress::begin_execution(db, &key, "processing", Some("target"), 2.0).unwrap());
    ingress::get(db, &key).unwrap().unwrap()
}

fn admit_intake(
    db: &Path,
    id: &str,
    event: i64,
) -> cdr_store::Result<intake::PromptIntakeAdmission> {
    intake::admit_prompt_intake(
        db,
        NewPromptIntake {
            job_id: id,
            target_thread_id: "target",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(event),
            raw_prompt: "original input",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: 3.0,
        },
    )
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let binding = json!({"target":"target","route":"Explicit",
            "command":{"Stop":{"reference":"target"}}});
        ingress::admit(&db, &NewIngress {
            ingress_id: "message:500".into(), kind: IngressKind::Message, event_id: Some(500),
            application_id: None, channel_id: 99, owner_user_id: 20,
            source_message_id: Some(500),
            payload: json!({"version":1,"plan":{"Execute":binding["command"]},"lifecycle_binding":binding}),
            target_thread_id: Some("target".into()), canonical_owner: None, now: 1.0,
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

    fn accept(&self, running: bool) -> cdr_store::Result<bool> {
        if running {
            Ok(control::accept_running(
                &self.db,
                scope(),
                &self.binding,
                Some(&self.stop),
                ("resident", 7),
                || Ok(()),
            )?
            .is_some())
        } else {
            Ok(accept_nonrunning(
                &self.db,
                scope(),
                &self.binding,
                Some(&self.stop),
                || Ok(()),
            )?
            .is_some())
        }
    }

    fn queued(&self, running: bool) {
        queue::enqueue(&self.db, job("queued-a", 102)).unwrap();
        if running {
            queue::begin_attempt(&self.db, "queued-a", &[], 7).unwrap();
            queue::mark_running(&self.db, "queued-a", "owned-turn", 7).unwrap();
        }
    }

    fn saved(&self, event: i64) -> StoredIngress {
        ingress::get(&self.db, &format!("message:{event}"))
            .unwrap()
            .unwrap()
    }

    fn assert_original(&self, before: &StoredIngress) {
        let saved = ingress::get(&self.db, &before.ingress_id).unwrap().unwrap();
        assert_eq!(saved.payload, before.payload);
        assert_eq!(saved.target_thread_id, before.target_thread_id);
        assert_eq!(saved.event_id, before.event_id);
        assert_eq!(saved.source_message_id, before.source_message_id);
        assert_eq!(saved.channel_id, before.channel_id);
        assert_eq!(saved.owner_user_id, before.owner_user_id);
        assert_eq!(saved.owner_id, None);
        assert_eq!(saved.state, "held");
        assert_eq!(
            saved.outcome.as_ref().unwrap()["stop_hold"]["ingress_id"],
            before.ingress_id
        );
    }

    fn assert_rollback(&self, before: &StoredIngress, queued: &[queue::StoredQueueJob]) {
        assert_eq!(self.saved(101), *before);
        assert_eq!(self.saved(500), self.stop);
        assert_eq!(queue::list_filtered(&self.db, None, None).unwrap(), queued);
        assert!(control::pending_after(&self.db, 0).unwrap().is_empty());
        let held: i64 = Connection::open(&self.db)
            .unwrap()
            .query_row("SELECT count(*) FROM cdr_execution_holds", [], |r| r.get(0))
            .unwrap();
        assert_eq!(held, 0);
    }
}

#[test]
fn unowned_stop_accepts_without_inventing_a_queue_job() {
    let f = Fixture::new();
    let before = prompt(&f.db, 101, 99, 20);
    assert!(
        f.accept(false).unwrap(),
        "original unowned request must be accepted"
    );
    f.assert_original(&before);
    assert!(queue::list_filtered(&f.db, None, None).unwrap().is_empty());
    assert!(intake::list_prompt_intakes(&f.db).unwrap().is_empty());
    let receipt = f.saved(500).outcome.unwrap();
    assert_eq!(receipt["jobs"], json!([]));
    assert_eq!(receipt["ingresses"], json!(["message:101"]));
    assert_eq!(receipt["execution_end_confirmed"], false);
}

#[test]
fn stopped_original_cannot_gain_late_intake_or_queue_ownership_after_reopen() {
    let f = Fixture::new();
    let before = prompt(&f.db, 101, 99, 20);
    assert!(f.accept(false).unwrap());
    f.assert_original(&before);
    assert!(admit_intake(&f.db, "late-a", 101).is_err());
    assert!(queue::enqueue(&f.db, job("different-late-id", 101)).is_err());
    assert!(intake::list_prompt_intakes(&f.db).unwrap().is_empty());
    assert!(queue::list_filtered(&f.db, None, None).unwrap().is_empty());
    assert!(
        ingress::begin_execution(&f.db, "message:101", "processing", Some("target"), 4.0)
            .is_ok_and(|changed| !changed)
    );
    assert!(
        ingress::record_result(&f.db, "message:101", &json!({"response":"late"}), 4.0).is_err()
    );
    f.assert_original(&before);
    let fresh = prompt(&f.db, 103, 99, 20);
    let new = admit_intake(&f.db, "fresh-b", 103).unwrap();
    assert!(new.created);
    assert_eq!(f.saved(103).owner_id.as_deref(), Some("fresh-b"));
    assert_eq!(f.saved(103).payload, fresh.payload);
}

#[test]
fn intake_ownership_before_stop_is_still_captured_by_existing_job_hold() {
    let f = Fixture::new();
    prompt(&f.db, 101, 99, 20);
    admit_intake(&f.db, "intake-a", 101).unwrap();
    let before = f.saved(101);
    assert!(f.accept(false).unwrap());
    assert_eq!(f.saved(101), before);
    assert!(
        cdr_store::execution_hold::reason(&f.db, "intake-a")
            .unwrap()
            .is_some()
    );
    assert!(
        intake::try_claim_prompt_intake(&f.db, "intake-a", 4.0, 604.0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn mixed_stop_preserves_unowned_original_and_running_interrupt_identity() {
    for running in [false, true] {
        let f = Fixture::new();
        let before = prompt(&f.db, 101, 99, 20);
        f.queued(running);
        let queued = queue::list_filtered(&f.db, None, None).unwrap();
        assert!(f.accept(running).unwrap());
        f.assert_original(&before);
        assert_eq!(queue::list_filtered(&f.db, None, None).unwrap(), queued);
        let receipt = f.saved(500).outcome.unwrap();
        assert_eq!(receipt["jobs"], json!(["queued-a"]));
        assert_eq!(receipt["ingresses"], json!(["message:101"]));
        if running {
            let controls = control::pending_after(&f.db, 0).unwrap();
            assert_eq!(controls.len(), 1);
            assert_eq!(controls[0].1.turn, "owned-turn");
            assert_eq!(controls[0].1.jobs.len(), 1);
        }
    }
}

#[test]
fn foreign_unowned_scope_refuses_both_stop_paths_without_partial_holds() {
    for running in [false, true] {
        for (channel, owner) in [(98, 20), (99, 21)] {
            let f = Fixture::new();
            let before = prompt(&f.db, 101, channel, owner);
            f.queued(running);
            let queued = queue::list_filtered(&f.db, None, None).unwrap();
            assert!(f.accept(running).is_err());
            f.assert_rollback(&before, &queued);
        }
    }
}

#[test]
fn failed_ignored_or_changed_ingress_write_rolls_back_stop_receipt_and_job_holds() {
    for running in [false, true] {
        for body in [
            "SELECT RAISE(ABORT,'injected unowned stop failure');",
            "SELECT RAISE(IGNORE);",
            "UPDATE discord_ingress_journal SET payload_json='{}' WHERE ingress_id='message:101';",
        ] {
            let f = Fixture::new();
            let before = prompt(&f.db, 101, 99, 20);
            f.queued(running);
            let queued = queue::list_filtered(&f.db, None, None).unwrap();
            Connection::open(&f.db)
                .unwrap()
                .execute_batch(&format!(
                "CREATE TRIGGER break_original BEFORE UPDATE OF state ON discord_ingress_journal
                 WHEN NEW.ingress_id='message:101' AND NEW.state='held' BEGIN {body} END;"
            ))
                .unwrap();
            assert!(f.accept(running).is_err());
            f.assert_rollback(&before, &queued);
        }
    }
}

#[test]
fn previous_ingress_hold_and_stop_receipt_cannot_be_overwritten_or_extended() {
    let f = Fixture::new();
    prompt(&f.db, 101, 99, 20);
    Connection::open(&f.db)
        .unwrap()
        .execute(
            "UPDATE discord_ingress_journal SET state='held',hold_reason='original unrelated hold',
         outcome_json='{\"original_evidence\":\"keep\"}' WHERE ingress_id='message:101'",
            [],
        )
        .unwrap();
    let before = f.saved(101);
    assert!(f.accept(false).unwrap());
    f.assert_original(&before);
    assert_eq!(f.saved(101).hold_reason, before.hold_reason);
    assert_eq!(f.saved(101).outcome.unwrap()["original_evidence"], "keep");
    ingress::record_result(&f.db, "message:500", &json!({"response":"accepted"}), 5.0).unwrap();
    let receipt = f.saved(500).outcome.unwrap()["stop_receipt"].clone();
    assert_eq!(receipt["ingresses"], json!(["message:101"]));
    let later = prompt(&f.db, 103, 99, 20);
    assert!(f.accept(false).is_err());
    assert_eq!(f.saved(103), later);
    ingress::record_result(
        &f.db,
        "message:500",
        &json!({"stop_receipt":{},"response":"later"}),
        6.0,
    )
    .unwrap();
    assert_eq!(f.saved(500).outcome.unwrap()["stop_receipt"], receipt);
}

#[test]
fn bounded_unowned_scope_never_partially_accepts() {
    let f = Fixture::new();
    for event in 1000..1129 {
        prompt(&f.db, event, 99, 20);
    }
    assert!(f.accept(false).is_err());
    assert_eq!(f.saved(500), f.stop);
    let held: i64 = Connection::open(&f.db)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM discord_ingress_journal WHERE state='held'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(held, 0);
}
