use super::*;
use crate::{prompt_intake, queue, reserve_policy};

fn job(id: &str, message: Option<i64>) -> queue::NewQueueJob<'_> {
    queue::NewQueueJob {
        job_id: id,
        target_thread_id: "thread",
        channel_id: 42,
        owner_user_id: Some(3),
        discord_message_id: message,
        app_server_generation: 1,
        prompt: "preserve input",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}
fn ingress(db: &Connection, id: &str, message: i64, accepted: bool) {
    db.execute(
        "INSERT INTO discord_ingress_journal
        (ingress_id,kind,event_id,channel_id,owner_user_id,payload_json,state,phase,
        target_thread_id,owner_kind,owner_id,confirmation_delivered,created_at,updated_at)
        VALUES (?,'message',?,42,3,'{}','owned','durable_prompt','thread','prompt',?,?,1,1)",
        params![format!("message:{message}"), message, id, accepted],
    )
    .unwrap();
}
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    open_initialized(&path).unwrap();
    (dir, path)
}

#[test]
fn retirement_uses_acceptance_not_ack_and_is_idempotent_across_reopen() {
    let (_dir, path) = fixture();
    let db = open_initialized(&path).unwrap();
    for (id, message, accepted) in [
        ("accepted", 11, true),
        ("failed", 12, true),
        ("unaccepted", 13, false),
    ] {
        queue::enqueue(&path, job(id, Some(message))).unwrap();
        ingress(&db, id, message, accepted);
    }
    queue::enqueue(&path, job("unproven-headless", None)).unwrap();
    queue::enqueue(&path, job("legacy-prefix", Some(14))).unwrap();
    ingress(&db, "legacy-prefix", 14, true);
    db.execute(
        "UPDATE codex_turn_queue SET last_error=? WHERE job_id='legacy-prefix'",
        [format!("{}old uncertainty", reserve_policy::HOLD_PREFIX)],
    )
    .unwrap();
    let key = json!([42, "message/error/v1", "inbound-message/12/error-report", 0]).to_string();
    crate::delivery_receipt::begin(&path, &key, "error-hash").unwrap();
    crate::delivery_receipt::confirm(&path, &key, "999").unwrap();
    reserve_policy::ensure(&path, "thread").unwrap();
    reserve_policy::mark_unknown(&path, "thread", "retain episode").unwrap();
    let before = rows(&db, "SELECT * FROM codex_reserve_policy").unwrap()[0].clone();
    let result = retire(&path, &[]).unwrap();
    assert_eq!(result.held_jobs, 4);
    assert!(execution_hold::reason(&path, "accepted").unwrap().is_none());
    for id in ["failed", "unaccepted", "unproven-headless", "legacy-prefix"] {
        assert!(execution_hold::reason(&path, id).unwrap().is_some());
        assert!(
            queue::try_begin_attempt(&path, id, &[], 1)
                .unwrap()
                .is_none()
        );
        assert!(queue::begin_attempt(&path, id, &[], 1).is_err());
    }
    let audit: String = db
        .query_row(
            "SELECT policy_json FROM cdr_reserve_retirement_evidence",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&audit).unwrap(), before);
    let mut after = rows(&db, "SELECT * FROM codex_reserve_policy").unwrap()[0].clone();
    assert_eq!(after["mode"], "manual");
    after["mode"] = before["mode"].clone();
    assert_eq!(after, before);
    queue::adopt_target_generation(&path, "thread", 2).unwrap();
    reserve_policy::set_mode(&path, "thread", "manual").unwrap();
    assert!(retire(&path, &[]).unwrap().already_completed);
    assert!(
        queue::try_begin_attempt(&path, "failed", &[], 2)
            .unwrap()
            .is_none()
    );
    assert!(
        queue::try_begin_attempt(&path, "accepted", &[], 2)
            .unwrap()
            .is_some()
    );
}

#[test]
fn retirement_failure_rolls_back_audit_mode_holds_and_completion_marker() {
    let (_dir, path) = fixture();
    let db = open_initialized(&path).unwrap();
    queue::enqueue(&path, job("unknown", None)).unwrap();
    reserve_policy::ensure(&path, "thread").unwrap();
    let before = reserve_policy::get(&path, "thread").unwrap();
    db.execute_batch("CREATE TRIGGER fail_retirement BEFORE INSERT ON cdr_store_retirements BEGIN SELECT RAISE(ABORT,'crash before commit'); END;").unwrap();
    assert!(retire(&path, &[]).is_err());
    assert_eq!(reserve_policy::get(&path, "thread").unwrap(), before);
    assert!(execution_hold::reason(&path, "unknown").unwrap().is_none());
    assert!(
        rows(&db, "SELECT * FROM cdr_reserve_retirement_evidence")
            .unwrap()
            .is_empty()
    );
    db.execute_batch("DROP TRIGGER fail_retirement").unwrap();
    assert_eq!(retire(&path, &[]).unwrap().held_jobs, 1);
    assert!(retire(&path, &[]).unwrap().already_completed);
}

#[test]
fn exact_positive_headless_classification_does_not_cover_discord_or_unknown_rows() {
    let (_dir, path) = fixture();
    let db = open_initialized(&path).unwrap();
    queue::enqueue(&path, job("headless", None)).unwrap();
    let evidence = HeadlessEvidence {
        job_id: "headless".into(),
        row_sha256: row_hash(&rows(&db, "SELECT * FROM codex_turn_queue").unwrap()[0]),
        provenance: "reviewed local admin submission receipt".into(),
    };
    queue::enqueue(&path, job("unknown", None)).unwrap();
    assert_eq!(retire(&path, &[evidence]).unwrap().held_jobs, 1);
    assert!(
        queue::try_begin_attempt(&path, "headless", &[], 1)
            .unwrap()
            .is_some()
    );
    assert!(
        queue::try_begin_attempt(&path, "unknown", &[], 1)
            .unwrap()
            .is_none()
    );
    let (_dir, other) = fixture();
    queue::enqueue(&other, job("discord", Some(11))).unwrap();
    let row = rows(
        &open_initialized(&other).unwrap(),
        "SELECT * FROM codex_turn_queue",
    )
    .unwrap()
    .remove(0);
    assert!(
        retire(
            &other,
            &[HeadlessEvidence {
                job_id: "discord".into(),
                row_sha256: row_hash(&row),
                provenance: "guess".into()
            }]
        )
        .is_err()
    );
}

#[test]
fn held_intake_cannot_claim_or_promote_using_an_old_lease() {
    let (_dir, path) = fixture();
    prompt_intake::admit_prompt_intake(
        &path,
        prompt_intake::NewPromptIntake {
            job_id: "intake",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            raw_prompt: "preserve input",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: 1.0,
        },
    )
    .unwrap();
    let claim = prompt_intake::try_claim_prompt_intake(&path, "intake", 2.0, 50.0)
        .unwrap()
        .unwrap();
    retire(&path, &[]).unwrap();
    assert!(
        prompt_intake::try_claim_prompt_intake(&path, "intake", 60.0, 90.0)
            .unwrap()
            .is_none()
    );
    assert!(
        prompt_intake::promote_prompt_intake_to_queue(&path, &claim, job("intake", None), 3.0)
            .is_err()
    );
    assert_eq!(prompt_intake::list_prompt_intakes(&path).unwrap().len(), 1);
    assert!(queue::list(&path).unwrap().is_empty());
}

#[test]
fn usage_rejection_atomically_preserves_hold_and_notice_without_policy() {
    let (_dir, path) = fixture();
    let db = open_initialized(&path).unwrap();
    queue::enqueue(&path, job("limited", None)).unwrap();
    let starting = queue::try_begin_attempt(&path, "limited", &[], 1)
        .unwrap()
        .unwrap();
    db.execute_batch("CREATE TRIGGER fail_notice BEFORE INSERT ON codex_reserve_start_notices BEGIN SELECT RAISE(ABORT,'notice failure'); END;").unwrap();
    let error = format!("{}typed usage limit", execution_hold::PREFIX);
    assert!(queue::record_start_failure_if_claimed(&path, &starting, &error, false).is_err());
    assert_eq!(queue::list(&path).unwrap()[0], starting);
    assert!(execution_hold::reason(&path, "limited").unwrap().is_none());
    db.execute_batch("DROP TRIGGER fail_notice").unwrap();
    queue::record_start_failure_if_claimed(&path, &starting, &error, false)
        .unwrap()
        .unwrap();
    assert!(reserve_policy::get(&path, "thread").unwrap().is_none());
    assert_eq!(
        reserve_policy::start_notice::pending(&path).unwrap().len(),
        1
    );
    assert!(
        queue::try_begin_attempt(&path, "limited", &[], 1)
            .unwrap()
            .is_none()
    );
}
