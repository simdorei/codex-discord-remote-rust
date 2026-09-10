use cdr_store::{
    prompt_intake::{self, NewPromptIntake},
    queue::{self, NewQueueJob},
    schema::open_initialized,
};
use std::{
    path::Path,
    sync::{Arc, Barrier},
};

fn intake(id: &str) -> NewPromptIntake<'_> {
    NewPromptIntake {
        job_id: id,
        target_thread_id: "original",
        channel_id: 42,
        owner_user_id: Some(3),
        discord_message_id: Some(101),
        raw_prompt: "original prompt",
        auto_queue_when_busy: true,
        require_current_mirror: false,
        created_at: 1.0,
    }
}
fn job(id: &str) -> NewQueueJob<'_> {
    NewQueueJob {
        job_id: id,
        target_thread_id: "original",
        channel_id: 42,
        owner_user_id: Some(3),
        discord_message_id: Some(101),
        app_server_generation: 1,
        prompt: "prepared",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}
fn cancel(db: &Path) -> cdr_store::Result<Option<String>> {
    queue::cancel_latest_pending(db, "original", 42, 3, 12.0)
}

#[test]
fn cancellation_survives_reopen_and_blocks_changed_job_id_with_same_event() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    prompt_intake::admit_prompt_intake(&db, intake("job")).unwrap();
    let claim = prompt_intake::try_claim_prompt_intake(&db, "job", 10.0, 20.0)
        .unwrap()
        .unwrap();
    assert_eq!(cancel(&db).unwrap().as_deref(), Some("job"));
    assert!(prompt_intake::promote_prompt_intake_to_queue(&db, &claim, job("job"), 13.0).is_err());
    for id in ["job", "different-job"] {
        assert!(prompt_intake::admit_prompt_intake(&db, intake(id)).is_err());
        assert!(queue::enqueue(&db, job(id)).is_err());
    }
    assert!(queue::list(&db).unwrap().is_empty());
    assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
}

#[test]
fn failed_receipt_or_removal_rolls_back_entire_cancellation() {
    for failure in ["receipt", "remove"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        let before = prompt_intake::admit_prompt_intake(&db, intake("job"))
            .unwrap()
            .intake;
        let trigger = if failure == "receipt" {
            "CREATE TRIGGER injected BEFORE INSERT ON codex_request_cancellations BEGIN SELECT RAISE(ABORT,'injected'); END"
        } else {
            "CREATE TRIGGER injected BEFORE DELETE ON codex_prompt_intakes BEGIN SELECT RAISE(ABORT,'injected'); END"
        };
        open_initialized(&db)
            .unwrap()
            .execute_batch(trigger)
            .unwrap();
        assert!(cancel(&db).is_err());
        assert_eq!(
            prompt_intake::get_prompt_intake(&db, "job").unwrap(),
            Some(before)
        );
        let receipts: i64 = open_initialized(&db)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM codex_request_cancellations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(receipts, 0);
    }
}

#[test]
fn cancellation_and_execution_claim_cannot_both_win() {
    for _ in 0..12 {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        prompt_intake::admit_prompt_intake(&db, intake("job")).unwrap();
        let claim = prompt_intake::try_claim_prompt_intake(&db, "job", 10.0, 20.0)
            .unwrap()
            .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let cancelled = {
            let (path, gate) = (db.clone(), barrier.clone());
            std::thread::spawn(move || {
                gate.wait();
                cancel(&path).is_ok_and(|v| v.is_some())
            })
        };
        barrier.wait();
        let started = prompt_intake::promote_prompt_intake_to_queue(&db, &claim, job("job"), 11.0)
            .is_ok()
            && queue::begin_attempt(&db, "job", &[], 1).is_ok();
        let cancelled = cancelled.join().unwrap();
        assert_ne!(
            started, cancelled,
            "one of cancellation or execution admission must win"
        );
        if cancelled {
            assert!(queue::list(&db).unwrap().is_empty());
        } else {
            assert_eq!(
                queue::list(&db).unwrap()[0].state,
                queue::QueueJobState::Starting
            );
        }
    }
}

#[test]
fn foreign_routes_and_conflicting_evidence_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    prompt_intake::admit_prompt_intake(&db, intake("job")).unwrap();
    for (target, channel, owner) in [("other", 42, 3), ("original", 43, 3), ("original", 42, 4)] {
        assert!(
            queue::cancel_latest_pending(&db, target, channel, owner, 12.0)
                .unwrap()
                .is_none()
        );
    }
    // A legacy partially promoted occurrence is not guessed to be unstarted.
    queue::enqueue(&db, job("job")).unwrap();
    assert!(cancel(&db).is_err());
    assert_eq!(queue::list(&db).unwrap().len(), 1);
    assert_eq!(prompt_intake::list_prompt_intakes(&db).unwrap().len(), 1);
}

#[test]
fn changed_mirror_is_rechecked_inside_cancellation_transaction() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    prompt_intake::admit_prompt_intake(&db, intake("job")).unwrap();
    cdr_store::mapping::upsert_thread(&db, "other", "project", "other", 100, 42, 1.0).unwrap();
    assert!(queue::cancel_latest_pending_on_route(&db, "original", 42, 3, 12.0, true).is_err());
    assert!(
        prompt_intake::get_prompt_intake(&db, "job")
            .unwrap()
            .is_some()
    );
    // An explicit original reference is not silently changed to the room target.
    assert_eq!(
        queue::cancel_latest_pending_on_route(&db, "original", 42, 3, 12.0, false)
            .unwrap()
            .as_deref(),
        Some("job")
    );
}
