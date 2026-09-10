use cdr_store::{ingress, prompt_intake, queue};
use serde_json::json;

fn seed(db: &std::path::Path) {
    ingress::admit(
        db,
        &ingress::NewIngress {
            ingress_id: "message:101".into(),
            kind: ingress::IngressKind::Message,
            event_id: Some(101),
            application_id: None,
            channel_id: 10,
            owner_user_id: 3,
            source_message_id: Some(101),
            payload: json!({"version":1,"plan":{"Execute":{"New":{"prompt":"first"}}}}),
            target_thread_id: None,
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    ingress::begin_thread_start(db, "message:101", 1, 2.0).unwrap();
    ingress::record_new_creation(db, "message:101", 1, Some("C:/fixture"), 10, 2.1).unwrap();
    ingress::record_created_thread(db, "message:101", 1, "created", 3.0).unwrap();
    cdr_store::mapping::upsert_thread(db, "created", "project", "new", 10, 42, 3.1).unwrap();
    prompt_intake::admit_prompt_intake_with_ingress(
        db,
        prompt_intake::NewPromptIntake {
            job_id: "new-job",
            target_thread_id: "created",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(101),
            raw_prompt: "first",
            auto_queue_when_busy: true,
            require_current_mirror: true,
            created_at: 4.0,
        },
        "message:101",
        1,
    )
    .unwrap();
}

#[test]
fn cross_channel_cancellation_requires_all_original_new_room_evidence() {
    for damage in [
        "UPDATE discord_ingress_journal SET outcome_json=json_set(outcome_json,'$.new_creation.version',2)",
        "UPDATE discord_ingress_journal SET outcome_json=json_set(outcome_json,'$.new_creation.origin_channel_id',99)",
        "UPDATE discord_ingress_journal SET outcome_json=json_remove(outcome_json,'$.new_creation')",
        "UPDATE discord_ingress_journal SET payload_json=json_set(payload_json,'$.version',2)",
        "UPDATE discord_ingress_journal SET payload_json=json('{\"version\":1,\"plan\":{\"Execute\":{\"Ask\":{\"prompt\":\"first\"}}}}')",
        "UPDATE discord_ingress_journal SET owner_user_id=99",
        "UPDATE discord_ingress_journal SET target_thread_id='other'",
        "UPDATE discord_ingress_journal SET owner_id='other-job'",
        "UPDATE discord_ingress_journal SET state='held'",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        seed(&db);
        let raw = rusqlite::Connection::open(&db).unwrap();
        raw.execute(damage, []).unwrap();
        let before = ingress::get(&db, "message:101").unwrap();
        assert!(
            queue::cancel_latest_pending(&db, "created", 42, 3, 5.0).is_err(),
            "{damage}"
        );
        assert_eq!(ingress::get(&db, "message:101").unwrap(), before);
        let (intakes, receipts): (i64, i64) = raw.query_row(
            "SELECT (SELECT COUNT(*) FROM codex_prompt_intakes),(SELECT COUNT(*) FROM codex_request_cancellations)",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!((intakes, receipts), (1, 0), "{damage}");
    }
}

#[test]
fn cancellation_survives_late_custody_drop_and_startup_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    queue::cancel_latest_pending(&db, "created", 42, 3, 5.0).unwrap();
    let before = ingress::get(&db, "message:101").unwrap();
    ingress::hold(&db, "message:101", "late worker drop", false, 6.0).unwrap();
    ingress::recover_prior_runtime(&db, "next-runtime", 7.0).unwrap();
    assert_eq!(ingress::get(&db, "message:101").unwrap(), before);
    let raw = rusqlite::Connection::open(&db).unwrap();
    let notices: i64 = raw
        .query_row("SELECT COUNT(*) FROM codex_delivery_outbox", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(notices, 0, "cancelled work is not unknown-outcome work");
}
