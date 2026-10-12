use cdr_store::{
    async_question as aq,
    ingress::stop::{StopScope, control},
    queue,
};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::{Value, json};
use std::path::PathBuf;

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    question: String,
    original: control::StopControl,
}
impl Fixture {
    fn new(submitted: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let question = fixture::dispatching(&db, "old-resident");
        if submitted {
            aq::confirm_dispatch(&db, &question, "original").unwrap();
        }
        aq::record_error(&db, &question, "original timeout evidence").unwrap();
        fixture::pending(&db, "old-pending", "thread-b", 1);
        let original = control::accept_running(&db,
            StopScope { target: "thread-b", channel: 20, owner: 30 },
            &json!({"target":"thread-b","route":"Explicit","command":{"Stop":{"reference":"thread-b"}}}),
            None, ("old-resident", 1), || Ok(())).unwrap().unwrap();
        Self {
            _temp: temp,
            db,
            question,
            original,
        }
    }
    fn observed() -> Value {
        json!({"threadId":"thread-b","truncated":false,"required_owners_complete":true,
            "turns":[{"id":"original","status":"interrupted"}],
            "thread_observation":{"thread":{"id":"thread-b","status":{"type":"notLoaded"}}},
            "goal_observation":{"goal":null}})
    }
    fn recover(&self, observed: &Value) -> cdr_store::Result<usize> {
        let mut db = Connection::open(&self.db)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result =
            control::recover_terminal_in(&tx, &self.original, observed, "new-resident", 9, 50.0)?;
        tx.commit()?;
        Ok(result)
    }
}

#[test]
fn stopped_original_settles_atomically_in_every_phase_and_preserves_policy_and_new_work() {
    for phase in [
        "accepted",
        "dispatching",
        "acknowledged",
        "unknown",
        "settled",
    ] {
        for publishing in [false, true] {
            let f = Fixture::new(phase != "accepted");
            let db = Connection::open(&f.db).unwrap();
            db.execute("UPDATE cdr_stop_controls SET phase=?", [phase])
                .unwrap();
            if publishing {
                db.execute(
                    "UPDATE cdr_async_execution_obligations SET policy='publishing_recovery'",
                    [],
                )
                .unwrap();
            }
            fixture::pending(&f.db, "fresh", "thread-b", 9);
            queue::adopt_target_generation(&f.db, "thread-b", 9).unwrap();
            let before_question = aq::get(&f.db, &f.question).unwrap();
            assert_eq!(control::recovery_after(&f.db, 0).unwrap().len(), 1);
            assert_eq!(f.recover(&Fixture::observed()).unwrap(), 2);
            assert!(!control::target_is_held(&f.db, "thread-b").unwrap());
            assert_eq!(
                queue::list(&f.db)
                    .unwrap()
                    .iter()
                    .map(|j| j.job_id.as_str())
                    .collect::<Vec<_>>(),
                ["fresh"]
            );
            assert_eq!(
                cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap(),
                publishing
            );
            let (execution, admission, answer): (String, String, String) = db.query_row(
                "SELECT execution_state,admission_state,answer_state FROM cdr_async_execution_obligations", [],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(execution, "terminal");
            assert_eq!(admission, if publishing { "held" } else { "settled" });
            assert_ne!(answer, "unresolved");
            let after_question = aq::get(&f.db, &f.question).unwrap();
            assert_eq!(after_question.error, before_question.error);
            assert_eq!(after_question.chosen, before_question.chosen);
            assert_eq!(after_question.body, before_question.body);
            let cancellations: i64 = db
                .query_row(
                    "SELECT count(*) FROM codex_request_cancellations",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(cancellations, 2);
            assert!(
                cdr_store::execution_hold::reason(&f.db, "origin")
                    .unwrap()
                    .is_some()
            );
            assert!(control::recovery_after(&f.db, 0).unwrap().is_empty());
        }
    }
}

#[test]
fn incomplete_or_different_terminal_evidence_keeps_original_queue_and_question() {
    for case in 0..9 {
        let f = Fixture::new(true);
        let before = queue::list(&f.db).unwrap();
        let mut observed = Fixture::observed();
        match case {
            0 => observed["turns"][0]["status"] = json!("inProgress"),
            1 => observed["turns"][0]["id"] = json!("replacement"),
            2 => observed["threadId"] = json!("other"),
            3 => observed["thread_observation"]["thread"]["status"]["type"] = json!("active"),
            4 => observed["goal_observation"] = json!({}),
            5 => {
                observed["goal_observation"]["goal"] =
                    json!({"threadId":"thread-b","status":"active"});
            }
            6 => {
                observed["goal_observation"]["goal"] =
                    json!({"threadId":"other","status":"complete"});
            }
            7 => observed["truncated"] = json!(true),
            _ => observed["turns"] = json!([]),
        }
        assert!(f.recover(&observed).is_err(), "case {case}");
        assert_eq!(queue::list(&f.db).unwrap(), before);
        assert!(control::target_is_held(&f.db, "thread-b").unwrap());
        assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
    }
}

#[test]
fn changed_original_or_missing_hold_cannot_be_cancelled_using_old_stop() {
    for sql in [
        "UPDATE codex_turn_queue SET prompt='changed' WHERE job_id='origin'",
        "DELETE FROM cdr_execution_holds WHERE job_id='origin'",
        "UPDATE cdr_stop_controls SET record_json=json_set(record_json,'$.owner',40)",
    ] {
        let f = Fixture::new(true);
        Connection::open(&f.db).unwrap().execute_batch(sql).unwrap();
        let before = queue::list(&f.db).unwrap();
        assert!(f.recover(&Fixture::observed()).is_err());
        assert_eq!(queue::list(&f.db).unwrap(), before);
        assert!(control::target_is_held(&f.db, "thread-b").unwrap());
    }
}

#[test]
fn a_failed_cancellation_certificate_or_stop_write_rolls_back_every_change() {
    for trigger in [
        "CREATE TRIGGER fault BEFORE DELETE ON codex_turn_queue BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fault BEFORE INSERT ON cdr_async_terminal_settlements BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fault BEFORE UPDATE ON cdr_stop_controls BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let f = Fixture::new(true);
        let db = Connection::open(&f.db).unwrap();
        db.execute_batch(trigger).unwrap();
        let before = queue::list(&f.db).unwrap();
        assert!(f.recover(&Fixture::observed()).is_err());
        assert_eq!(queue::list(&f.db).unwrap(), before);
        assert!(control::target_is_held(&f.db, "thread-b").unwrap());
        assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM codex_request_cancellations",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}

#[test]
fn queue_absence_alone_is_not_terminal_proof() {
    let f = Fixture::new(true);
    queue::complete(&f.db, "origin").unwrap();
    queue::complete(&f.db, "old-pending").unwrap();
    assert!(f.recover(&json!({})).is_err());
    assert!(control::target_is_held(&f.db, "thread-b").unwrap());
    assert_eq!(f.recover(&Fixture::observed()).unwrap(), 0);
    assert!(!control::target_is_held(&f.db, "thread-b").unwrap());
    assert!(!cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
}
