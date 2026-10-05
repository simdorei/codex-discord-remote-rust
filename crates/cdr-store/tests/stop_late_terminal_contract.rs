use std::path::PathBuf;

use cdr_store::{
    ingress::stop::{StopScope, accept_unresolved, control},
    prompt_intake::{self, NewPromptIntake},
    queue::{self, NewQueueJob, StoredQueueJob},
};
use rusqlite::{Connection, types::Value as SqlValue};
use serde_json::{Value, json};

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    claimed: StoredQueueJob,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("late-terminal.sqlite");
        enqueue(&db, "original", 700);
        let claimed = queue::begin_attempt(&db, "original", &[], 7).unwrap();
        Self {
            _temp: temp,
            db,
            claimed,
        }
    }

    fn stop(&self) {
        accept_unresolved(
            &self.db,
            StopScope { target: "target", channel: 99, owner: 20 },
            &json!({"target":"target","route":"Explicit","command":{"Stop":{"reference":"target"}}}),
            None,
            || Ok(()),
        ).unwrap().unwrap();
    }

    fn ack(&self) -> control::StopControl {
        queue::mark_running_with_resident_if_claimed(
            &self.db,
            &self.claimed,
            "original-turn",
            Some("original-resident"),
        )
        .unwrap()
        .unwrap();
        control::pending_after(&self.db, 0).unwrap().remove(0).1
    }

    fn terminal(&self, target: &str, turn: &str, generation: i64, resident: &str, status: &str) {
        cdr_store::observed_completion::record_for_resident(
            &self.db,
            target,
            turn,
            generation,
            &json!({"threadId":target,"turn":{"id":turn,"status":status}}).to_string(),
            resident,
        )
        .unwrap();
    }

    fn exact_terminal(&self) {
        self.terminal(
            "target",
            "original-turn",
            7,
            "original-resident",
            "interrupted",
        );
    }

    fn held(&self) -> bool {
        control::target_is_held(&self.db, "target").unwrap()
    }

    fn phase(&self, original: &control::StopControl) -> String {
        control::phase(&self.db, &original.operation_id)
            .unwrap()
            .unwrap()
    }

    // All columns of original execution custody and accepted revision evidence.
    // Terminal observations and the changing control phase are deliberately separate.
    fn custody(&self) -> Vec<Vec<Vec<SqlValue>>> {
        let db = Connection::open(&self.db).unwrap();
        [
            "codex_turn_queue",
            "cdr_execution_holds",
            "cdr_stop_revision_receipts",
            "cdr_stop_revisions",
            "cdr_stop_clock",
            "codex_prompt_intakes",
        ]
        .iter()
        .map(|table| {
            let mut statement = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get::<_, SqlValue>(column))
                        .collect()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        })
        .collect()
    }
}

fn enqueue(db: &std::path::Path, job: &str, event: i64) {
    queue::enqueue(
        db,
        NewQueueJob {
            job_id: job,
            target_thread_id: "target",
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

#[test]
fn late_single_original_settles_only_after_exact_terminal() {
    for status in ["completed", "interrupted", "failed"] {
        let f = Fixture::new();
        f.stop();
        let original = f.ack();
        let custody = f.custody();
        assert!(f.held(), "a late start ACK is not terminal evidence");
        assert_eq!(f.phase(&original), "accepted");
        f.terminal("target", "original-turn", 7, "original-resident", status);
        assert!(
            !f.held(),
            "the exact terminal must settle a single original late start"
        );
        assert_eq!(f.phase(&original), "settled");
        assert!(original.can_settle);
        assert_eq!(
            f.custody(),
            custody,
            "settlement cannot rewrite or replay original work"
        );
        assert!(
            cdr_store::execution_hold::reason(&f.db, "original")
                .unwrap()
                .is_some()
        );
        f.terminal("target", "original-turn", 7, "original-resident", status);
        assert_eq!(f.phase(&original), "settled");
        assert_eq!(f.custody(), custody);
        assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    }
}

#[test]
fn unrelated_terminal_identity_cannot_close_late_stop() {
    for (target, turn, generation, resident) in [
        ("other-target", "original-turn", 7, "original-resident"),
        ("target", "other-turn", 7, "original-resident"),
        ("target", "original-turn", 8, "original-resident"),
        ("target", "original-turn", 7, "other-resident"),
    ] {
        let f = Fixture::new();
        f.stop();
        let original = f.ack();
        let custody = f.custody();
        f.terminal(target, turn, generation, resident, "interrupted");
        assert!(f.held());
        assert_eq!(f.phase(&original), "accepted");
        assert_eq!(f.custody(), custody);
        f.exact_terminal();
        assert!(
            !f.held(),
            "only the exact resident/generation/turn/target may settle"
        );
        assert_eq!(f.custody(), custody);
    }
}

#[test]
fn mixed_original_queue_scope_stays_unresolved() {
    for starting in [false, true] {
        let f = Fixture::new();
        enqueue(&f.db, "another-original", 701);
        if starting {
            queue::begin_attempt(&f.db, "another-original", &[], 7).unwrap();
        }
        f.stop();
        let original = f.ack();
        let custody = f.custody();
        assert!(!original.can_settle);
        f.exact_terminal();
        assert!(f.held(), "one terminal cannot dispose the other original");
        assert_eq!(f.phase(&original), "unknown");
        assert_eq!(f.custody(), custody);
    }
}

#[test]
fn unresolved_intake_is_not_a_single_original() {
    for job in ["original", "preparing-original"] {
        let f = Fixture::new();
        prompt_intake::admit_prompt_intake(
            &f.db,
            NewPromptIntake {
                job_id: job,
                target_thread_id: "target",
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(702),
                raw_prompt: "preparing input",
                auto_queue_when_busy: true,
                require_current_mirror: false,
                created_at: 1.0,
            },
        )
        .unwrap();
        f.stop();
        let original = f.ack();
        let custody = f.custody();
        assert!(
            !original.can_settle,
            "queue/intake ID deduplication must not erase uncertainty"
        );
        f.exact_terminal();
        assert!(f.held());
        assert_eq!(f.phase(&original), "unknown");
        assert_eq!(f.custody(), custody);
    }
}

#[test]
fn incomplete_or_nonempty_ingress_scope_never_grants_settlement() {
    for replacement in [
        None,
        Some(Value::Null),
        Some(json!({})),
        Some(json!(["unresolved-ingress"])),
    ] {
        let f = Fixture::new();
        f.stop();
        let db = Connection::open(&f.db).unwrap();
        let text: String = db
            .query_row(
                "SELECT scope_json FROM cdr_stop_revision_receipts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut scope: Value = serde_json::from_str(&text).unwrap();
        if let Some(value) = replacement {
            scope["ingresses"] = value;
        } else {
            scope.as_object_mut().unwrap().remove("ingresses");
        }
        // Injected evidence corruption, not an actual ingress admission scenario.
        db.execute(
            "UPDATE cdr_stop_revision_receipts SET scope_json=?",
            [scope.to_string()],
        )
        .unwrap();
        let original = f.ack();
        let custody = f.custody();
        assert!(!original.can_settle);
        f.exact_terminal();
        assert!(f.held());
        assert_eq!(f.phase(&original), "unknown");
        assert_eq!(f.custody(), custody);
    }
}

#[test]
fn original_intake_cleanup_cannot_create_single_scope_authority() {
    for cleanup_before_ack in [true, false] {
        let f = Fixture::new();
        prompt_intake::admit_prompt_intake(
            &f.db,
            NewPromptIntake {
                job_id: "original",
                target_thread_id: "target",
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(702),
                raw_prompt: "separate preparing input",
                auto_queue_when_busy: true,
                require_current_mirror: false,
                created_at: 1.0,
            },
        )
        .unwrap();
        f.stop();
        let db = Connection::open(&f.db).unwrap();
        let receipt: String = db
            .query_row(
                "SELECT scope_json FROM cdr_stop_revision_receipts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let original = if cleanup_before_ack {
            assert!(prompt_intake::remove_prompt_intake_if_queued(&f.db, "original").unwrap());
            f.ack()
        } else {
            let original = f.ack();
            assert!(prompt_intake::remove_prompt_intake_if_queued(&f.db, "original").unwrap());
            original
        };
        assert!(
            prompt_intake::get_prompt_intake(&f.db, "original")
                .unwrap()
                .is_none()
        );
        let after: String = db
            .query_row(
                "SELECT scope_json FROM cdr_stop_revision_receipts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after, receipt, "cleanup cannot rewrite the accepted scope");
        let custody = f.custody();
        f.exact_terminal();
        assert!(
            f.held(),
            "cleanup is not proof that the original preparing uncertainty ended"
        );
        assert!(!original.can_settle);
        assert_eq!(f.phase(&original), "unknown");
        assert_eq!(f.custody(), custody);
        assert!(
            cdr_store::execution_hold::reason(&f.db, "original")
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn accepted_preparing_absence_must_be_explicit_and_valid() {
    for replacement in [
        None,
        Some(Value::Null),
        Some(json!(true)),
        Some(json!("false")),
        Some(json!([])),
    ] {
        let f = Fixture::new();
        f.stop();
        let db = Connection::open(&f.db).unwrap();
        let text: String = db
            .query_row(
                "SELECT scope_json FROM cdr_stop_revision_receipts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut scope: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            scope["hadPreparing"], false,
            "new receipts retain original composition"
        );
        if let Some(value) = replacement {
            scope["hadPreparing"] = value;
        } else {
            scope.as_object_mut().unwrap().remove("hadPreparing");
        }
        // Historical/malformed evidence is never upgraded by current absence.
        db.execute(
            "UPDATE cdr_stop_revision_receipts SET scope_json=?",
            [scope.to_string()],
        )
        .unwrap();
        let original = f.ack();
        let custody = f.custody();
        assert!(!original.can_settle);
        f.exact_terminal();
        assert!(f.held());
        assert_eq!(f.phase(&original), "unknown");
        assert_eq!(f.custody(), custody);
    }
}

#[test]
fn missing_running_match_or_permanent_hold_keeps_stop_unresolved() {
    for sql in [
        "DELETE FROM cdr_execution_holds WHERE job_id='original'",
        "UPDATE codex_turn_queue SET turn_id='different-turn' WHERE job_id='original'",
    ] {
        let f = Fixture::new();
        f.stop();
        let original = f.ack();
        Connection::open(&f.db).unwrap().execute_batch(sql).unwrap();
        let custody = f.custody();
        f.exact_terminal();
        assert!(
            f.held(),
            "incomplete matching execution evidence cannot release target"
        );
        assert_ne!(f.phase(&original), "settled");
        assert_eq!(f.custody(), custody);
    }
}
