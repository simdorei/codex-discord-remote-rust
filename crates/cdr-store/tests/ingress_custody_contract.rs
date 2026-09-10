use cdr_store::ingress::{self, IngressKind, NewIngress};
use cdr_store::prompt_intake::{self, NewPromptIntake};
use cdr_store::{dead_generation, delivery, processed, schema};
use serde_json::json;

fn request(id: i64) -> NewIngress {
    NewIngress {
        ingress_id: format!("message:{id}"),
        kind: IngressKind::Message,
        event_id: Some(id),
        application_id: Some(99),
        channel_id: 10,
        owner_user_id: 20,
        source_message_id: Some(id),
        payload: json!({"content":"보존할 원문","plan":{"Ask":{"prompt":"preserve me"}}}),
        target_thread_id: Some("thread-a".into()),
        canonical_owner: None,
        now: 100.0,
    }
}

fn intake(id: i64) -> NewPromptIntake<'static> {
    NewPromptIntake {
        job_id: "owned-job",
        target_thread_id: "thread-a",
        channel_id: 10,
        owner_user_id: Some(20),
        discord_message_id: Some(id),
        raw_prompt: "preserve me",
        auto_queue_when_busy: true,
        require_current_mirror: false,
        created_at: 101.0,
    }
}

#[test]
fn ig_03_claim_write_failure_rolls_back_the_original_payload_too() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let connection = schema::open_initialized(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_claim BEFORE INSERT ON discord_processed_messages BEGIN SELECT RAISE(ABORT,'fixture claim rejected'); END;").unwrap();
    let error = ingress::admit(&db, &request(501)).err().unwrap();
    assert!(error.to_string().contains("fixture claim rejected"));
    assert!(ingress::get(&db, "message:501").unwrap().is_none());
    assert!(!processed::is_processed(&db, 501).unwrap());
    assert!(delivery::list_pending(&db).unwrap().is_empty());
}

#[test]
fn ig_03_journal_write_failure_never_creates_a_processed_claim() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let connection = schema::open_initialized(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_journal BEFORE INSERT ON discord_ingress_journal BEGIN SELECT RAISE(ABORT,'fixture journal rejected'); END;").unwrap();
    assert!(ingress::admit(&db, &request(502)).is_err());
    assert!(!processed::is_processed(&db, 502).unwrap());
}

#[test]
fn legacy_id_only_claim_stays_non_replayable_without_fabricated_contents() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    assert!(processed::claim(&db, 503, 90.0).unwrap());
    let admitted = ingress::admit(&db, &request(503)).unwrap();
    assert!(!admitted.created);
    assert!(admitted.record.is_none());
    assert!(ingress::get(&db, "message:503").unwrap().is_none());
}

#[test]
fn event_repeat_cannot_change_payload_or_its_original_owner() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let original = request(504);
    assert!(ingress::admit(&db, &original).unwrap().created);
    let mut repeated = request(504);
    repeated.payload = json!({"content":"replacement must not overwrite"});
    assert!(!ingress::admit(&db, &repeated).unwrap().created);
    assert_eq!(
        ingress::get(&db, "message:504").unwrap().unwrap().payload,
        original.payload
    );
    repeated.owner_user_id = 21;
    assert!(ingress::admit(&db, &repeated).is_err());
    assert_eq!(
        ingress::get(&db, "message:504")
            .unwrap()
            .unwrap()
            .owner_user_id,
        20
    );
}

#[test]
fn ig_05_handoff_failure_rolls_back_intake_and_permanent_owner_together() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    ingress::admit(&db, &request(505)).unwrap();
    let connection = schema::open_initialized(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_owner BEFORE UPDATE OF owner_id ON discord_ingress_journal BEGIN SELECT RAISE(ABORT,'fixture owner rejected'); END;").unwrap();
    assert!(prompt_intake::admit_prompt_intake(&db, intake(505)).is_err());
    assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
    assert!(
        ingress::get(&db, "message:505")
            .unwrap()
            .unwrap()
            .owner_id
            .is_none()
    );
    connection
        .execute_batch("DROP TRIGGER reject_owner;")
        .unwrap();
    prompt_intake::admit_prompt_intake(&db, intake(505)).unwrap();
    assert_eq!(
        ingress::get(&db, "message:505")
            .unwrap()
            .unwrap()
            .owner_id
            .as_deref(),
        Some("owned-job")
    );
}

#[test]
fn ig_05_completed_transient_queue_does_not_erase_the_permanent_ingress_owner() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    dead_generation::activate_runtime(&db, "runtime-a").unwrap();
    ingress::admit(&db, &request(506)).unwrap();
    prompt_intake::admit_prompt_intake(&db, intake(506)).unwrap();
    let claim = prompt_intake::try_claim_prompt_intake(&db, "owned-job", 102.0, 200.0)
        .unwrap()
        .unwrap();
    prompt_intake::promote_prompt_intake_to_queue(
        &db,
        &claim,
        cdr_store::queue::NewQueueJob {
            job_id: "owned-job",
            target_thread_id: "thread-a",
            channel_id: 10,
            owner_user_id: Some(20),
            discord_message_id: Some(506),
            app_server_generation: 1,
            prompt: "prepared",
            queued: true,
            ack_sent: false,
            created_at: 102.0,
        },
        103.0,
    )
    .unwrap();
    cdr_store::queue::complete(&db, "owned-job").unwrap();
    dead_generation::activate_runtime(&db, "runtime-b").unwrap();
    assert_eq!(
        ingress::recover_prior_runtime(&db, "runtime-b", 201.0).unwrap(),
        0
    );
    let stored = ingress::get(&db, "message:506").unwrap().unwrap();
    assert_eq!(stored.state, "owned");
    assert_eq!(stored.owner_id.as_deref(), Some("owned-job"));
    assert!(delivery::list_pending(&db).unwrap().is_empty());
    assert!(!ingress::admit(&db, &request(506)).unwrap().created);
}

#[test]
fn ig_06_restart_holds_unowned_work_and_keeps_a_failed_notice_without_reexecution() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    dead_generation::activate_runtime(&db, "runtime-a").unwrap();
    ingress::admit(&db, &request(507)).unwrap();
    dead_generation::activate_runtime(&db, "runtime-b").unwrap();
    assert_eq!(
        ingress::recover_prior_runtime(&db, "runtime-b", 200.0).unwrap(),
        1
    );
    let record = ingress::get(&db, "message:507").unwrap().unwrap();
    assert_eq!(record.state, "held");
    assert_eq!(record.payload["content"], "보존할 원문");
    let notices = delivery::list_pending(&db).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.contains("not executed"));
    assert!(notices[0].content.contains("message:507"));
    assert!(!notices[0].content.contains("보존할 원문"));
    delivery::record_failure(&db, &notices[0].delivery_id, "fixture HTTP 503", 201.0).unwrap();
    assert_eq!(
        ingress::recover_prior_runtime(&db, "runtime-b", 202.0).unwrap(),
        0
    );
    assert_eq!(delivery::list_pending(&db).unwrap()[0].attempt_count, 1);
    delivery::complete(&db, &notices[0].delivery_id).unwrap();
    dead_generation::activate_runtime(&db, "runtime-c").unwrap();
    assert_eq!(
        ingress::recover_prior_runtime(&db, "runtime-c", 300.0).unwrap(),
        0
    );
    assert!(delivery::list_pending(&db).unwrap().is_empty());
    assert!(!ingress::begin_execution(&db, "message:507", "processing", None, 301.0).unwrap());
    assert!(cdr_store::queue::list(&db).unwrap().is_empty());
}

#[test]
fn ig_08_completed_action_with_missing_confirmation_is_not_unknown_or_reexecutable() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    dead_generation::activate_runtime(&db, "runtime-a").unwrap();
    ingress::admit(&db, &request(508)).unwrap();
    ingress::begin_execution(&db, "message:508", "processing", Some("thread-a"), 101.0).unwrap();
    ingress::record_result(&db, "message:508", &json!({"result":"completed"}), 102.0).unwrap();
    dead_generation::activate_runtime(&db, "runtime-b").unwrap();
    ingress::recover_prior_runtime(&db, "runtime-b", 200.0).unwrap();
    let notice = delivery::list_pending(&db).unwrap().remove(0);
    assert!(notice.content.contains("action completed"));
    assert!(!notice.content.contains("outcome is unknown"));
    assert!(!ingress::begin_execution(&db, "message:508", "processing", None, 201.0).unwrap());
}
