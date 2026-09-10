use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    prompt_intake::{self, NewPromptIntake},
    queue,
};
use serde_json::json;

fn seed(db: &std::path::Path) {
    ingress::admit(
        db,
        &NewIngress {
            ingress_id: "message:101".into(),
            kind: IngressKind::Message,
            event_id: Some(101),
            application_id: None,
            channel_id: 42,
            owner_user_id: 3,
            source_message_id: Some(101),
            payload: json!({"original":"keep this"}),
            target_thread_id: Some("original".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    ingress::begin_execution(db, "message:101", "processing", None, 2.0).unwrap();
    prompt_intake::admit_prompt_intake(
        db,
        NewPromptIntake {
            job_id: "job",
            target_thread_id: "original",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(101),
            raw_prompt: "keep this",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: 2.0,
        },
    )
    .unwrap();
    ingress::record_result(
        db,
        "message:101",
        &json!({"new_creation":{"original_id":"original"},"new_verification":{"confirmed":true}}),
        3.0,
    )
    .unwrap();
}

#[test]
fn cancellation_preserves_existing_creation_evidence_and_original_payload() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    let before = ingress::get(&db, "message:101").unwrap().unwrap();
    queue::cancel_latest_pending(&db, "original", 42, 3, 4.0).unwrap();
    let after = ingress::get(&db, "message:101").unwrap().unwrap();
    assert_eq!(after.payload, before.payload);
    for field in ["new_creation", "new_verification"] {
        assert_eq!(
            after.outcome.as_ref().and_then(|v| v.get(field)),
            before.outcome.as_ref().and_then(|v| v.get(field)),
            "{field}"
        );
    }
    assert_eq!(after.phase, "cancelled");
    assert_eq!(after.outcome.unwrap()["kind"], "request_cancelled");
}

#[test]
fn late_worker_result_cannot_erase_a_durable_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    queue::cancel_latest_pending(&db, "original", 42, 3, 4.0).unwrap();
    let before = ingress::get(&db, "message:101").unwrap().unwrap();
    assert!(
        ingress::record_result(&db, "message:101", &json!({"action_completed":true}), 5.0).is_err()
    );
    assert_eq!(ingress::get(&db, "message:101").unwrap(), Some(before));
    ingress::confirm(&db, "message:101", 6.0).unwrap();
    let confirmed = ingress::get(&db, "message:101").unwrap().unwrap();
    assert_eq!(confirmed.phase, "cancelled");
    assert!(confirmed.confirmation_delivered);
}

#[test]
fn first_new_prompt_can_be_cancelled_from_its_verified_new_room() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    ingress::admit(
        &db,
        &NewIngress {
            ingress_id: "message:101".into(),
            kind: IngressKind::Message,
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
    ingress::begin_thread_start(&db, "message:101", 1, 2.0).unwrap();
    ingress::record_new_creation(&db, "message:101", 1, Some("C:/fixture"), 10, 2.1).unwrap();
    ingress::record_created_thread(&db, "message:101", 1, "created", 3.0).unwrap();
    cdr_store::mapping::upsert_thread(&db, "created", "project", "new", 10, 42, 3.1).unwrap();
    prompt_intake::admit_prompt_intake_with_ingress(
        &db,
        NewPromptIntake {
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
    assert_eq!(
        queue::cancel_latest_pending_on_route(&db, "created", 42, 3, 5.0, true)
            .unwrap()
            .as_deref(),
        Some("new-job")
    );
    let saved = ingress::get(&db, "message:101").unwrap().unwrap();
    assert_eq!(
        saved.channel_id, 10,
        "original envelope channel is immutable"
    );
    assert_eq!(saved.phase, "cancelled");
    assert!(
        saved
            .outcome
            .as_ref()
            .unwrap()
            .get("new_creation")
            .is_some()
    );
    assert!(
        ingress::record_result(&db, "message:101", &json!({"action_completed":true}), 6.0).is_err()
    );
}

#[test]
fn mapped_prompt_can_be_cancelled_before_a_worker_starts_intake() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    ingress::admit(
        &db,
        &NewIngress {
            ingress_id: "message:201".into(),
            kind: IngressKind::Message,
            event_id: Some(201),
            application_id: None,
            channel_id: 42,
            owner_user_id: 3,
            source_message_id: Some(201),
            payload: json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"pending"}}}}),
            target_thread_id: Some("original".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    assert!(
        queue::cancel_latest_pending(&db, "original", 42, 3, 2.0)
            .unwrap()
            .is_some()
    );
    assert!(!ingress::begin_execution(&db, "message:201", "processing", None, 3.0).unwrap());
    let record = ingress::get(&db, "message:201").unwrap().unwrap();
    assert_eq!(record.phase, "cancelled");
    assert_eq!(
        record.payload["plan"]["Execute"]["Ask"]["prompt"],
        "pending"
    );
    assert!(
        prompt_intake::admit_prompt_intake(
            &db,
            NewPromptIntake {
                job_id: "late-job",
                target_thread_id: "original",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(201),
                raw_prompt: "pending",
                auto_queue_when_busy: true,
                require_current_mirror: false,
                created_at: 3.0,
            }
        )
        .is_err()
    );
}
