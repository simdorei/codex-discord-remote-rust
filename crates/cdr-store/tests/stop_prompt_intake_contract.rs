use std::path::PathBuf;

use cdr_store::{
    ingress::{
        self, IngressKind, NewIngress, StoredIngress,
        stop::{StopScope, accept_nonrunning, control},
    },
    prompt_intake::{self as intake, NewPromptIntake, StoredPromptIntake},
    queue::{self, NewQueueJob},
};
use rusqlite::Connection;
use serde_json::{Value, json};

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    binding: Value,
    ingress: StoredIngress,
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
        prompt: "prepared input",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        Self::add_intake(&db, "intake-a", 101, 99, 20);
        let binding = json!({"target":"target","route":"Explicit",
            "command":{"Stop":{"reference":"target"}}});
        ingress::admit(&db, &NewIngress {
            ingress_id: "message:500".into(), kind: IngressKind::Message, event_id: Some(500),
            application_id: None, channel_id: 99, owner_user_id: 20, source_message_id: Some(500),
            payload: json!({"version":1,"plan":{"Execute":binding["command"]},"lifecycle_binding":binding}),
            target_thread_id: Some("target".into()), canonical_owner: None, now: 2.0,
        }).unwrap();
        assert!(
            ingress::begin_execution(&db, "message:500", "processing", Some("target"), 3.0)
                .unwrap()
        );
        let record = ingress::get(&db, "message:500").unwrap().unwrap();
        Self {
            _temp: temp,
            db,
            binding,
            ingress: record,
        }
    }

    fn add_intake(db: &std::path::Path, id: &str, event: i64, channel: i64, owner: i64) {
        intake::admit_prompt_intake(
            db,
            NewPromptIntake {
                job_id: id,
                target_thread_id: "target",
                channel_id: channel,
                owner_user_id: Some(owner),
                discord_message_id: Some(event),
                raw_prompt: "original raw input",
                auto_queue_when_busy: true,
                require_current_mirror: false,
                created_at: 1.0,
            },
        )
        .unwrap();
    }

    fn original(&self) -> StoredPromptIntake {
        intake::get_prompt_intake(&self.db, "intake-a")
            .unwrap()
            .unwrap()
    }

    fn accept(&self) -> cdr_store::Result<Option<cdr_store::ingress::stop::StopReceipt>> {
        accept_nonrunning(
            &self.db,
            scope(),
            &self.binding,
            Some(&self.ingress),
            || Ok(()),
        )
    }

    fn assert_unaccepted(&self, before: &StoredPromptIntake) {
        assert_eq!(self.original(), *before);
        assert_eq!(
            ingress::get(&self.db, "message:500").unwrap().as_ref(),
            Some(&self.ingress)
        );
        let count: i64 = Connection::open(&self.db)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM cdr_execution_holds", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn intake_only_stop_preserves_original_and_blocks_new_claim() {
    let f = Fixture::new();
    let before = f.original();
    let receipt = f
        .accept()
        .unwrap()
        .expect("owned preparing request must be accepted");
    assert_eq!(receipt.jobs, ["intake-a"]);
    assert_eq!(f.original(), before);
    assert!(
        intake::try_claim_prompt_intake(&f.db, "intake-a", 4.0, 604.0)
            .unwrap()
            .is_none()
    );
    assert!(queue::list_filtered(&f.db, None, None).unwrap().is_empty());
}

#[test]
fn claimed_intake_cannot_promote_after_stop_or_startup_lease_release() {
    let f = Fixture::new();
    let claim = intake::try_claim_prompt_intake(&f.db, "intake-a", 4.0, 604.0)
        .unwrap()
        .unwrap();
    let before = f.original();
    f.accept().unwrap().expect("claimed intake must be held");
    assert_eq!(f.original(), before);
    assert!(
        intake::promote_prompt_intake_to_queue(&f.db, &claim, job("intake-a", 101), 5.0).is_err()
    );
    assert_eq!(f.original(), before);
    assert_eq!(intake::release_all_prompt_intake_claims(&f.db).unwrap(), 1);
    assert!(
        intake::try_claim_prompt_intake(&f.db, "intake-a", 605.0, 1205.0)
            .unwrap()
            .is_none()
    );
    assert!(
        intake::promote_prompt_intake_to_queue(&f.db, &claim, job("intake-a", 101), 606.0).is_err()
    );
    let reopened = Connection::open(&f.db).unwrap();
    let count: i64 = reopened
        .query_row(
            "SELECT count(*) FROM cdr_execution_holds WHERE job_id='intake-a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(f.original().raw_prompt, before.raw_prompt);
    assert!(queue::list_filtered(&f.db, None, None).unwrap().is_empty());
}

#[test]
fn mixed_pending_and_preparing_receipt_is_retained_and_cannot_capture_later_intake() {
    let f = Fixture::new();
    queue::enqueue(&f.db, job("queued-a", 102)).unwrap();
    let before = queue::list_filtered(&f.db, None, None).unwrap();
    let original = f.original();
    assert_eq!(f.accept().unwrap().unwrap().jobs, ["intake-a", "queued-a"]);
    ingress::record_result(
        &f.db,
        "message:500",
        &json!({"response":"accepted","waits_for_final":false}),
        5.0,
    )
    .unwrap();
    ingress::confirm(&f.db, "message:500", 6.0).unwrap();
    let saved = ingress::get(&f.db, "message:500").unwrap().unwrap();
    assert_eq!(
        saved.outcome.as_ref().unwrap()["stop_receipt"]["jobs"],
        json!(["intake-a", "queued-a"])
    );
    assert_eq!(
        saved.outcome.as_ref().unwrap()["stop_receipt"]["execution_end_confirmed"],
        false
    );
    Fixture::add_intake(&f.db, "later", 103, 99, 20);
    assert!(f.accept().is_err());
    assert!(
        cdr_store::execution_hold::reason(&f.db, "later")
            .unwrap()
            .is_none()
    );
    assert_eq!(queue::list_filtered(&f.db, None, None).unwrap(), before);
    assert_eq!(f.original(), original);
}

#[test]
fn running_stop_holds_preparation_without_inventing_an_interrupt_turn() {
    let f = Fixture::new();
    queue::enqueue(&f.db, job("running-a", 102)).unwrap();
    queue::begin_attempt(&f.db, "running-a", &[], 7).unwrap();
    queue::mark_running(&f.db, "running-a", "owned-turn", 7).unwrap();
    let before = queue::list_filtered(&f.db, None, None).unwrap();
    let original = f.original();
    let accepted = control::accept_running(
        &f.db,
        scope(),
        &f.binding,
        Some(&f.ingress),
        ("resident", 7),
        || Ok(()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(accepted.turn, "owned-turn");
    assert_eq!(accepted.jobs.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&accepted.jobs[0]).unwrap()["job_id"],
        "running-a"
    );
    assert_eq!(queue::list_filtered(&f.db, None, None).unwrap(), before);
    assert_eq!(f.original(), original);
    assert!(
        cdr_store::execution_hold::reason(&f.db, "intake-a")
            .unwrap()
            .is_some()
    );
    assert!(
        intake::try_claim_prompt_intake(&f.db, "intake-a", 5.0, 605.0)
            .unwrap()
            .is_none()
    );
    let saved = ingress::get(&f.db, "message:500").unwrap().unwrap();
    assert_eq!(
        saved.outcome.as_ref().unwrap()["jobs"],
        json!(["intake-a", "running-a"])
    );
    assert_eq!(
        control::phase(&f.db, &accepted.operation_id)
            .unwrap()
            .as_deref(),
        Some("accepted")
    );
}

#[test]
fn foreign_preparing_owner_or_channel_rejects_nonrunning_and_running_atomically() {
    for running in [false, true] {
        for (channel, owner) in [(98, 20), (99, 21)] {
            let f = Fixture::new();
            queue::enqueue(&f.db, job("queued-a", 102)).unwrap();
            if running {
                queue::begin_attempt(&f.db, "queued-a", &[], 7).unwrap();
                queue::mark_running(&f.db, "queued-a", "owned-turn", 7).unwrap();
            }
            Fixture::add_intake(&f.db, "foreign", 103, channel, owner);
            let before = f.original();
            let queue_before = queue::list_filtered(&f.db, None, None).unwrap();
            if running {
                assert!(
                    control::accept_running(
                        &f.db,
                        scope(),
                        &f.binding,
                        Some(&f.ingress),
                        ("resident", 7),
                        || Ok(())
                    )
                    .is_err()
                );
            } else {
                assert!(f.accept().is_err());
            }
            f.assert_unaccepted(&before);
            assert_eq!(
                queue::list_filtered(&f.db, None, None).unwrap(),
                queue_before
            );
            assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
        }
    }
}

#[test]
fn failed_ignored_or_mutating_intake_hold_rolls_back_all_acceptance() {
    for running in [false, true] {
        for trigger in [
            "CREATE TRIGGER fail_hold BEFORE INSERT ON cdr_execution_holds WHEN NEW.job_id='intake-a' BEGIN SELECT RAISE(ABORT,'injected'); END;",
            "CREATE TRIGGER ignore_hold BEFORE INSERT ON cdr_execution_holds WHEN NEW.job_id='intake-a' BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER mutate_intake AFTER INSERT ON cdr_execution_holds WHEN NEW.job_id='intake-a' BEGIN UPDATE codex_prompt_intakes SET raw_prompt='changed' WHERE job_id='intake-a'; END;",
        ] {
            let f = Fixture::new();
            queue::enqueue(&f.db, job("queued-a", 102)).unwrap();
            if running {
                queue::begin_attempt(&f.db, "queued-a", &[], 7).unwrap();
                queue::mark_running(&f.db, "queued-a", "owned-turn", 7).unwrap();
            }
            let before = f.original();
            let queued = queue::list_filtered(&f.db, None, None).unwrap();
            Connection::open(&f.db)
                .unwrap()
                .execute_batch(trigger)
                .unwrap();
            if running {
                assert!(
                    control::accept_running(
                        &f.db,
                        scope(),
                        &f.binding,
                        Some(&f.ingress),
                        ("resident", 7),
                        || Ok(())
                    )
                    .is_err()
                );
            } else {
                assert!(f.accept().is_err());
            }
            f.assert_unaccepted(&before);
            assert_eq!(queue::list_filtered(&f.db, None, None).unwrap(), queued);
            assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
        }
    }
}

#[test]
fn preexisting_intake_hold_evidence_is_preserved() {
    let f = Fixture::new();
    let db = Connection::open(&f.db).unwrap();
    db.execute("INSERT INTO cdr_execution_holds VALUES('intake-a','target','prior','opaque original evidence',1)", []).unwrap();
    f.accept()
        .unwrap()
        .expect("existing original hold must be included in receipt");
    let saved: (String, String, f64) = db.query_row(
        "SELECT reason,evidence_json,created_at FROM cdr_execution_holds WHERE job_id='intake-a'",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap();
    assert_eq!(
        saved,
        ("prior".into(), "opaque original evidence".into(), 1.0)
    );
}

#[test]
fn bounded_intake_set_refuses_partial_acceptance() {
    let f = Fixture::new();
    for index in 0..128 {
        Fixture::add_intake(&f.db, &format!("extra-{index}"), 1000 + index, 99, 20);
    }
    let before = f.original();
    assert!(f.accept().is_err());
    f.assert_unaccepted(&before);
    assert_eq!(intake::list_prompt_intakes(&f.db).unwrap().len(), 129);
}

#[test]
fn promotion_before_stop_is_captured_as_original_queue_not_reissued() {
    let f = Fixture::new();
    let claim = intake::try_claim_prompt_intake(&f.db, "intake-a", 4.0, 604.0)
        .unwrap()
        .unwrap();
    intake::promote_prompt_intake_to_queue(&f.db, &claim, job("intake-a", 101), 5.0).unwrap();
    let queued = queue::list_filtered(&f.db, None, None).unwrap();
    assert_eq!(f.accept().unwrap().unwrap().jobs, ["intake-a"]);
    assert_eq!(queue::list_filtered(&f.db, None, None).unwrap(), queued);
    assert!(
        intake::get_prompt_intake(&f.db, "intake-a")
            .unwrap()
            .is_none()
    );
    assert!(
        cdr_store::execution_hold::reason(&f.db, "intake-a")
            .unwrap()
            .is_some()
    );
}
