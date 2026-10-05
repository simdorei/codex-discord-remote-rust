use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    prompt_intake::{self, NewPromptIntake},
    queue::{self, NewQueueJob},
    schema::open_initialized,
};
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Barrier},
};
fn seed(db: &Path, id: &str, event: i64, target: &str, owner: i64) {
    queue::enqueue(
        db,
        NewQueueJob {
            job_id: id,
            target_thread_id: target,
            channel_id: 42,
            owner_user_id: Some(owner),
            discord_message_id: Some(event),
            app_server_generation: 1,
            prompt: "preserve original",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
}
#[test]
fn active_uncertain_and_pending_requests_cancel_with_evidence_and_no_replay() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db, "pending", 1, "A", 3);
    seed(&db, "starting", 2, "A", 3);
    seed(&db, "running", 3, "A", 3);
    seed(&db, "other", 4, "B", 3);
    queue::try_begin_attempt(&db, "starting", &[], 1).unwrap();
    queue::try_begin_attempt(&db, "running", &[], 1).unwrap();
    queue::mark_running(&db, "running", "original-turn", 1).unwrap();
    let result = queue::cancel_for_recovery(&db, "A", 42, 3, 10.0).unwrap();
    assert_eq!(result.jobs.len(), 3);
    assert_eq!(result.started_or_uncertain, 2);
    assert_eq!(queue::list(&db).unwrap()[0].job_id, "other");
    let connection = open_initialized(&db).unwrap();
    let evidence: String = connection
        .query_row(
            "SELECT evidence_json FROM cdr_execution_holds WHERE job_id='running'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(evidence.contains("original-turn") && evidence.contains("preserve original"));
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM codex_request_cancellations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 3);
    assert!(queue::mark_running(&db, "starting", "late-ack", 1).is_err());
    assert!(
        queue::try_begin_attempt(&db, "pending", &[], 1)
            .unwrap()
            .is_none()
    );
    let duplicate = NewQueueJob {
        job_id: "different-id",
        target_thread_id: "A",
        channel_id: 42,
        owner_user_id: Some(3),
        discord_message_id: Some(3),
        app_server_generation: 2,
        prompt: "do not replay",
        queued: true,
        ack_sent: true,
        created_at: 20.0,
    };
    assert!(queue::enqueue(&db, duplicate).is_err());
    assert!(
        queue::cancel_for_recovery(&db, "A", 42, 3, 20.0)
            .unwrap()
            .jobs
            .is_empty()
    );
}
#[test]
fn other_sender_or_channel_rolls_back_the_entire_cancellation() {
    for (channel, owner) in [(43, 3), (42, 4)] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        seed(&db, "own", 1, "A", 3);
        seed(&db, "foreign", 2, "A", 4);
        assert!(queue::cancel_for_recovery(&db, "A", channel, owner, 10.0).is_err());
        assert_eq!(queue::list(&db).unwrap().len(), 2);
        let count: i64 = open_initialized(&db)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM codex_request_cancellations",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
#[test]
fn claim_race_cannot_restore_a_cancelled_request() {
    for _ in 0..8 {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        seed(&db, "job", 1, "A", 3);
        let barrier = Arc::new(Barrier::new(2));
        let other = barrier.clone();
        let path = db.clone();
        let claim = std::thread::spawn(move || {
            other.wait();
            queue::try_begin_attempt(&path, "job", &[], 1).unwrap()
        });
        barrier.wait();
        assert_eq!(
            queue::cancel_for_recovery(&db, "A", 42, 3, 10.0)
                .unwrap()
                .jobs
                .len(),
            1
        );
        let _ = claim.join().unwrap();
        assert!(queue::list(&db).unwrap().is_empty());
        assert!(
            queue::try_begin_attempt(&db, "job", &[], 1)
                .unwrap()
                .is_none()
        );
    }
}
fn unowned(db: &Path, payload: serde_json::Value) {
    ingress::admit(
        db,
        &NewIngress {
            ingress_id: "message:5".into(),
            kind: IngressKind::Message,
            event_id: Some(5),
            application_id: None,
            channel_id: 42,
            owner_user_id: 3,
            source_message_id: Some(5),
            payload,
            target_thread_id: Some("A".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    ingress::begin_execution(db, "message:5", "preparing", None, 2.0).unwrap();
}
#[test]
fn executing_preparation_and_intake_are_cancelled_without_late_promotion() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    unowned(
        &db,
        json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"p"}}}}),
    );
    prompt_intake::admit_prompt_intake(
        &db,
        NewPromptIntake {
            job_id: "intake",
            target_thread_id: "A",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(6),
            raw_prompt: "intake evidence",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: 3.0,
        },
    )
    .unwrap();
    let result = queue::cancel_for_recovery(&db, "A", 42, 3, 10.0).unwrap();
    assert_eq!(result.jobs.len(), 2);
    assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
    assert_eq!(
        ingress::get(&db, "message:5").unwrap().unwrap().phase,
        "cancelled"
    );
    assert!(!ingress::begin_execution(&db, "message:5", "late preparation", None, 20.0).unwrap());
}
#[test]
fn non_prompt_control_and_unknown_envelopes_are_preserved() {
    for payload in [
        json!({"version":true,"plan":{"Execute":{"Ask":{"prompt":"p"}}}}),
        json!({"version":1,"plan":{"Execute":{"Recover":{"reference":null}}}}),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        unowned(&db, payload);
        let before = ingress::get(&db, "message:5").unwrap();
        assert!(
            queue::cancel_for_recovery(&db, "A", 42, 3, 10.0)
                .unwrap()
                .jobs
                .is_empty()
        );
        assert_eq!(ingress::get(&db, "message:5").unwrap(), before);
    }
}
