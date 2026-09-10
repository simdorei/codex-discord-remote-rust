use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    prompt_intake::{self, NewPromptIntake},
};
use serde_json::json;

const RAW: &str = "original";
const PREPARED: &str =
    "original\nDiscord attachments saved locally:\npath: fixture.txt\nsha256: fixture";

fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    ingress::admit(
        &db,
        &NewIngress {
            ingress_id: "message:30".into(),
            kind: IngressKind::Message,
            event_id: Some(30),
            application_id: None,
            channel_id: 10,
            owner_user_id: 20,
            source_message_id: Some(30),
            target_thread_id: None,
            canonical_owner: None,
            now: 1.0,
            payload: json!({"version":1,"content":RAW,
            "attachments":[{"id":31,"filename":"fixture.txt","url":"http://127.0.0.1/fixture.txt"}],
            "plan":{"Execute":{"New":{"prompt":RAW}}}}),
        },
    )
    .unwrap();
    assert!(ingress::begin_execution(&db, "message:30", "processing", None, 2.0).unwrap());
    (temp, db)
}

fn prepare(db: &std::path::Path) {
    ingress::record_new_input(db, "message:30", RAW, PREPARED, 3.0).unwrap();
}

#[test]
fn missing_attachment_preparation_rejects_thread_start_without_changing_state() {
    let (_temp, db) = setup();
    assert!(ingress::begin_thread_start(&db, "message:30", 1, 4.0).is_err());
    let saved = ingress::get(&db, "message:30").unwrap().unwrap();
    assert_eq!(saved.phase, "processing");
    assert!(ingress::new_execution_prompt(&saved).is_err());
}

#[test]
fn prepared_input_is_write_once_and_survives_generation_and_result_updates() {
    let (_temp, db) = setup();
    let original = ingress::get(&db, "message:30").unwrap().unwrap().payload;
    prepare(&db);
    prepare(&db);
    assert!(ingress::record_new_input(&db, "message:30", RAW, "replacement", 4.0).is_err());
    assert!(ingress::record_new_input(&db, "message:30", "changed raw", PREPARED, 4.0).is_err());
    assert!(ingress::begin_thread_start(&db, "message:30", 1, 5.0).unwrap());
    ingress::record_created_thread(&db, "message:30", 1, "created", 6.0).unwrap();
    ingress::record_result(
        &db,
        "message:30",
        &json!({"response":"done","new_input":{"prompt":"forged"}}),
        7.0,
    )
    .unwrap();
    let saved = ingress::get(&db, "message:30").unwrap().unwrap();
    assert_eq!(saved.payload, original);
    assert_eq!(ingress::new_command_prompt(&saved), Some(RAW));
    assert_eq!(
        ingress::new_execution_prompt(&saved).unwrap(),
        Some(PREPARED)
    );
}

#[test]
fn changed_original_envelope_or_prepared_input_rejects_start() {
    for update in [
        "payload_json=json_set(payload_json,'$.attachments[0].id',99)",
        "payload_json=json_set(payload_json,'$.attachments[0].url','http://127.0.0.1/changed')",
        "payload_json=json_set(payload_json,'$.plan.Execute.New.prompt','changed')",
        "payload_json=json_set(payload_json,'$.attachments',json('[]'))",
        "payload_json=json_remove(payload_json,'$.plan.Execute.New')",
        "owner_user_id=99",
        "channel_id=99",
        "event_id=99",
        "outcome_json=json_set(outcome_json,'$.new_input.prompt','changed')",
        "outcome_json=json_set(outcome_json,'$.new_input.source_sha256','changed')",
        "outcome_json=json_set(outcome_json,'$.new_input.version',2)",
    ] {
        let (_temp, db) = setup();
        prepare(&db);
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute(
                &format!(
                    "UPDATE discord_ingress_journal SET {update} WHERE ingress_id='message:30'"
                ),
                [],
            )
            .unwrap();
        assert!(
            ingress::begin_thread_start(&db, "message:30", 1, 4.0).is_err(),
            "{update}"
        );
        assert_eq!(
            ingress::get(&db, "message:30").unwrap().unwrap().phase,
            "processing"
        );
    }
}

#[test]
fn prepared_attachment_prompt_is_the_only_allowed_new_room_handoff() {
    for (prompt, event, owner, valid) in [
        (PREPARED, 30, 20, true),
        (RAW, 30, 20, false),
        (PREPARED, 31, 20, false),
        (PREPARED, 30, 21, false),
    ] {
        let (_temp, db) = setup();
        prepare(&db);
        assert!(ingress::begin_thread_start(&db, "message:30", 1, 4.0).unwrap());
        ingress::record_created_thread(&db, "message:30", 1, "created", 5.0).unwrap();
        cdr_store::mapping::upsert_thread(&db, "created", "cwd", "new", 10, 11, 5.0).unwrap();
        let result = prompt_intake::admit_prompt_intake_with_ingress(
            &db,
            NewPromptIntake {
                job_id: "job",
                target_thread_id: "created",
                channel_id: 11,
                owner_user_id: Some(owner),
                discord_message_id: Some(event),
                raw_prompt: prompt,
                auto_queue_when_busy: true,
                require_current_mirror: true,
                created_at: 6.0,
            },
            "message:30",
            1,
        );
        assert_eq!(result.is_ok(), valid, "{result:?}");
        assert_eq!(
            prompt_intake::list_prompt_intakes(&db).unwrap().len(),
            usize::from(valid)
        );
        assert_eq!(
            ingress::get(&db, "message:30").unwrap().unwrap().channel_id,
            10
        );
    }
}
