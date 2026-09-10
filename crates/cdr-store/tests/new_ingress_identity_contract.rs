use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    prompt_intake::{self, NewPromptIntake},
};
use serde_json::json;

#[test]
fn same_room_new_handoff_rejects_changed_prompt_and_origin_event() {
    for changed_event in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        ingress::admit(
            &db,
            &NewIngress {
                ingress_id: "action:new".into(),
                kind: IngressKind::Action,
                event_id: Some(30),
                application_id: None,
                channel_id: 10,
                owner_user_id: 20,
                source_message_id: Some(30),
                payload: json!({"command":"new","prompt":"original"}),
                target_thread_id: None,
                canonical_owner: None,
                now: 1.0,
            },
        )
        .unwrap();
        assert!(ingress::begin_thread_start(&db, "action:new", 1, 2.0).unwrap());
        ingress::record_created_thread(&db, "action:new", 1, "created", 3.0).unwrap();
        let result = prompt_intake::admit_prompt_intake_with_ingress(
            &db,
            NewPromptIntake {
                job_id: "job",
                target_thread_id: "created",
                channel_id: 10,
                owner_user_id: Some(20),
                discord_message_id: Some(if changed_event { 31 } else { 30 }),
                raw_prompt: if changed_event {
                    "original"
                } else {
                    "replacement"
                },
                auto_queue_when_busy: true,
                require_current_mirror: false,
                created_at: 4.0,
            },
            "action:new",
            1,
        );
        assert!(
            result.is_err(),
            "same-room handoff must still verify original prompt and event"
        );
        assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
        let saved = ingress::get(&db, "action:new").unwrap().unwrap();
        assert_eq!(saved.phase, "thread/created");
        assert!(saved.owner_id.is_none());
    }
}

#[test]
fn new_prompt_parser_is_transport_and_version_specific() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    for (i, kind, payload, expected) in [
        (
            1,
            IngressKind::Action,
            json!({"command":"new","prompt":"a"}),
            Some("a"),
        ),
        (
            2,
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":{"New":{"prompt":"b"}}}}),
            Some("b"),
        ),
        (
            3,
            IngressKind::Interaction,
            json!({"version":1,"work":{"Slash":{"name":"new","values":{"prompt":{"String":"c"}}}}}),
            Some("c"),
        ),
        (
            4,
            IngressKind::Message,
            json!({"command":"new","prompt":"fake"}),
            None,
        ),
        (
            5,
            IngressKind::Message,
            json!({"version":2,"plan":{"Execute":{"New":{"prompt":"future"}}}}),
            None,
        ),
        (
            6,
            IngressKind::Interaction,
            json!({"version":1,"work":{"Slash":{"name":"steer","values":{"prompt":{"String":"wrong command"}}}}}),
            None,
        ),
        (
            7,
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":{"New":{"prompt":4}}}}),
            None,
        ),
    ] {
        let key = format!("fixture:{i}");
        let stored = ingress::admit(
            &db,
            &NewIngress {
                ingress_id: key,
                kind,
                event_id: Some(i),
                application_id: None,
                channel_id: 10,
                owner_user_id: 20,
                source_message_id: None,
                payload,
                target_thread_id: None,
                canonical_owner: None,
                now: 1.0,
            },
        )
        .unwrap()
        .record
        .unwrap();
        assert_eq!(ingress::new_command_prompt(&stored), expected);
    }
}
