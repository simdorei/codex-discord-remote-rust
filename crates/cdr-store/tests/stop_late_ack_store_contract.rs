use cdr_store::{
    ingress::stop::{StopScope, accept_unresolved, control, revision},
    queue::{self, NewQueueJob, QueueJobState, StoredQueueJob},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    claimed: StoredQueueJob,
}
impl Fixture {
    fn new(baseline: &[String]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        queue::enqueue(
            &db,
            NewQueueJob {
                job_id: "original",
                target_thread_id: "target",
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(700),
                app_server_generation: 7,
                prompt: "original input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        let claimed = queue::begin_attempt(&db, "original", baseline, 7).unwrap();
        Self {
            _temp: temp,
            db,
            claimed,
        }
    }
    fn stop(&self) {
        accept_unresolved(&self.db,StopScope{target:"target",channel:99,owner:20},
            &json!({"target":"target","route":"Explicit","command":{"Stop":{"reference":"target"}}}),
            None,||Ok(())).unwrap().unwrap();
    }
    fn ack(&self, turn: &str) -> cdr_store::Result<Option<StoredQueueJob>> {
        queue::mark_running_with_resident_if_claimed(
            &self.db,
            &self.claimed,
            turn,
            Some("original-resident"),
        )
    }
    fn row(&self) -> StoredQueueJob {
        queue::list(&self.db).unwrap().remove(0)
    }
    fn receipt(&self) -> String {
        Connection::open(&self.db)
            .unwrap()
            .query_row(
                "SELECT scope_json FROM cdr_stop_revision_receipts",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
}

#[test]
fn altered_ack_turn_cannot_become_original_control_authority() {
    for (owned, stopped) in [(true, true), (true, false), (false, true), (false, false)] {
        let f = Fixture::new(&[]);
        if stopped {
            f.stop();
        }
        let receipt = stopped.then(|| f.receipt());
        let hold = cdr_store::execution_hold::reason(&f.db, "original").unwrap();
        Connection::open(&f.db)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER replace_ack_turn AFTER UPDATE OF turn_id ON codex_turn_queue
             WHEN NEW.job_id = 'original' AND NEW.state = 'running'
                  AND NEW.turn_id = 'original-turn'
             BEGIN
                 UPDATE codex_turn_queue SET turn_id = 'replacement-turn'
                 WHERE job_id = NEW.job_id;
             END;",
            )
            .unwrap();
        let result = if owned {
            f.ack("original-turn")
        } else {
            queue::mark_running_if_claimed(&f.db, &f.claimed, "original-turn")
        };
        assert!(
            result.is_err(),
            "owned={owned}, stopped={stopped}: substituted ACK accepted"
        );
        assert_eq!(
            f.row(),
            f.claimed,
            "the entire original Starting must survive"
        );
        assert_eq!(stopped.then(|| f.receipt()), receipt);
        assert_eq!(
            cdr_store::execution_hold::reason(&f.db, "original").unwrap(),
            hold
        );
        assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    }
}

#[test]
fn late_ack_and_exact_original_control_commit_together_once() {
    let f = Fixture::new(&[]);
    f.stop();
    let receipt = f.receipt();
    let original_hold = cdr_store::execution_hold::reason(&f.db, "original").unwrap();
    let running = f.ack("new-original-turn").unwrap().unwrap();
    assert_eq!(running.state, QueueJobState::Running);
    assert_eq!(running.prompt, f.claimed.prompt);
    assert_eq!(running.attempt_count, f.claimed.attempt_count);
    let controls = control::pending_after(&f.db, 0).unwrap();
    assert_eq!(controls.len(), 1);
    let c = &controls[0].1;
    assert_eq!(c.resident, "original-resident");
    assert_eq!(c.generation, 7);
    assert_eq!(c.turn, "new-original-turn");
    assert_eq!(c.jobs, [serde_json::to_string(&running).unwrap()]);
    assert!(
        c.can_settle,
        "the single original is eligible for exact terminal settlement"
    );
    assert!(
        control::target_is_held(&f.db, "target").unwrap(),
        "ACK alone is not execution end"
    );
    assert_eq!(f.receipt(), receipt);
    assert_eq!(
        cdr_store::execution_hold::reason(&f.db, "original").unwrap(),
        original_hold
    );
    assert!(
        f.ack("other-turn").unwrap().is_none(),
        "original CAS is single-use"
    );
    assert_eq!(control::pending_after(&f.db, 0).unwrap(), controls);
    assert_eq!(f.row(), running);
}

#[test]
fn legacy_ack_without_resident_never_invents_interrupt_ownership() {
    let f = Fixture::new(&[]);
    f.stop();
    assert!(
        queue::mark_running_if_claimed(&f.db, &f.claimed, "known-turn")
            .unwrap()
            .is_some()
    );
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    assert!(
        cdr_store::execution_hold::reason(&f.db, "original")
            .unwrap()
            .is_some()
    );
}

#[test]
fn an_ordinary_ack_does_not_create_a_stop() {
    let f = Fixture::new(&[]);
    assert!(f.ack("normal-turn").unwrap().is_some());
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    assert_eq!(
        revision::capture(&f.db, Some("target")).unwrap()["stopRevision"],
        0
    );
}

#[test]
fn failed_ignored_or_corrupted_control_insert_rolls_back_the_ack() {
    for (timing, body) in [
        (
            "BEFORE",
            "SELECT RAISE(ABORT,'injected control insert failure');",
        ),
        ("BEFORE", "SELECT RAISE(IGNORE);"),
        ("AFTER", "UPDATE cdr_stop_controls SET phase='settled';"),
        (
            "AFTER",
            "UPDATE cdr_stop_controls SET resident_owner='replacement';",
        ),
        ("AFTER", "DELETE FROM cdr_execution_holds;"),
        (
            "AFTER",
            "UPDATE codex_turn_queue SET turn_id='replacement-turn';",
        ),
        (
            "AFTER",
            "UPDATE cdr_stop_revision_receipts SET scope_json='{}';",
        ),
    ] {
        let f = Fixture::new(&[]);
        f.stop();
        let receipt = f.receipt();
        Connection::open(&f.db).unwrap().execute_batch(&format!(
            "CREATE TRIGGER reject_late_ack {timing} INSERT ON cdr_stop_controls BEGIN {body} END;"
        )).unwrap();
        assert!(f.ack("original-turn").is_err(), "{timing}: {body}");
        assert_eq!(
            f.row(),
            f.claimed,
            "ack and control must roll back together"
        );
        assert_eq!(f.receipt(), receipt);
        assert!(
            cdr_store::execution_hold::reason(&f.db, "original")
                .unwrap()
                .is_some()
        );
        assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    }
}

#[test]
fn stale_claim_or_removed_original_never_creates_a_control() {
    let f = Fixture::new(&[]);
    f.stop();
    let mut stale = f.claimed.clone();
    stale.attempt_count += 1;
    assert!(
        queue::mark_running_with_resident_if_claimed(
            &f.db,
            &stale,
            "new-turn",
            Some("original-resident")
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(f.row(), f.claimed);
    assert!(queue::complete(&f.db, "original").unwrap());
    assert!(f.ack("new-turn").unwrap().is_none());
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    assert!(
        cdr_store::execution_hold::reason(&f.db, "original")
            .unwrap()
            .is_some()
    );
}

#[test]
fn changed_original_actor_or_prompt_cannot_borrow_the_ack() {
    for actor in [true, false] {
        let f = Fixture::new(&[]);
        f.stop();
        let mut claim = f.claimed.clone();
        if actor {
            claim.owner_user_id = Some(21);
        } else {
            claim.prompt = "replacement input".into();
        }
        assert!(
            queue::mark_running_with_resident_if_claimed(
                &f.db,
                &claim,
                "original-turn",
                Some("original-resident")
            )
            .is_err()
        );
        assert_eq!(f.row(), f.claimed);
        assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    }
}

#[test]
fn baseline_turn_or_invalid_resident_does_not_gain_new_control_authority() {
    for (turn, resident) in [("prior-turn", "original-resident"), ("new-turn", " ")] {
        let f = Fixture::new(&["prior-turn".into()]);
        f.stop();
        assert!(
            queue::mark_running_with_resident_if_claimed(&f.db, &f.claimed, turn, Some(resident))
                .is_err()
        );
        assert_eq!(f.row(), f.claimed);
        assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    }
}

#[test]
fn a_receipt_excluding_this_original_is_not_widened() {
    let f = Fixture::new(&[]);
    f.stop();
    // Inject an incomplete stored scope in this temporary DB; it is not a new
    // stop operation. The original hold stays; missing scope cannot grant control.
    let db = Connection::open(&f.db).unwrap();
    let mut scope: Value = serde_json::from_str(&f.receipt()).unwrap();
    scope["jobs"] = json!([]);
    db.execute(
        "UPDATE cdr_stop_revision_receipts SET scope_json=?",
        [scope.to_string()],
    )
    .unwrap();
    let receipt = f.receipt();
    assert!(f.ack("new-turn").unwrap().is_some());
    assert_eq!(f.receipt(), receipt);
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    assert!(
        cdr_store::execution_hold::reason(&f.db, "original")
            .unwrap()
            .is_some()
    );
}
