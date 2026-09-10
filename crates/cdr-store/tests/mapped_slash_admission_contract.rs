use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    mapping,
    prompt_intake::{self, NewPromptIntake},
};
use serde_json::json;

fn request() -> NewIngress {
    NewIngress {
        ingress_id: "interaction:201".into(),
        kind: IngressKind::Interaction,
        event_id: Some(201),
        application_id: Some(2),
        channel_id: 42,
        owner_user_id: 3,
        source_message_id: None,
        payload: json!({"version":1,"work":{"Slash":{"name":"ask","values":{"prompt":{"String":"original"}}}}}),
        target_thread_id: None,
        canonical_owner: Some("interaction:201".into()),
        now: 1.0,
    }
}

#[test]
fn mapping_change_cannot_rebind_admitted_slash_or_its_later_intake() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    mapping::upsert_thread(&db, "original", "project", "old", 100, 42, 1.0).unwrap();
    let input = request();
    let admitted = ingress::admit_mapped_slash_prompt(&db, &input)
        .unwrap()
        .record
        .unwrap();
    assert_eq!(admitted.payload, input.payload);
    assert_eq!(admitted.target_thread_id.as_deref(), Some("original"));
    // Move the original away, then associate the old room with another thread.
    mapping::upsert_thread(&db, "original", "project", "old", 100, 43, 2.0).unwrap();
    mapping::upsert_thread(&db, "other", "project", "other", 100, 42, 2.0).unwrap();
    assert!(
        !ingress::admit_mapped_slash_prompt(&db, &input)
            .unwrap()
            .created
    );
    assert_eq!(
        ingress::get(&db, "interaction:201").unwrap(),
        Some(admitted.clone())
    );
    assert!(ingress::begin_execution(&db, "interaction:201", "processing", None, 3.0).is_err());
    assert_eq!(
        ingress::get(&db, "interaction:201").unwrap(),
        Some(admitted.clone())
    );
    assert!(
        prompt_intake::admit_prompt_intake(
            &db,
            NewPromptIntake {
                job_id: "wrong-target",
                target_thread_id: "other",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(201),
                raw_prompt: "original",
                auto_queue_when_busy: true,
                require_current_mirror: true,
                created_at: 4.0,
            }
        )
        .is_err()
    );
    assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
    assert_eq!(
        ingress::get(&db, "interaction:201").unwrap(),
        Some(admitted)
    );
}

#[test]
fn unsupported_envelopes_are_not_given_a_guessed_target() {
    for (version, name) in [
        (json!(true), "ask"),
        (json!(2), "ask"),
        (json!(1), "new"),
        (json!(1), "help"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        mapping::upsert_thread(&db, "original", "project", "old", 100, 42, 1.0).unwrap();
        let mut input = request();
        input.payload["version"] = version;
        input.payload["work"]["Slash"]["name"] = name.into();
        assert!(ingress::admit_mapped_slash_prompt(&db, &input).is_err());
        assert!(ingress::get(&db, "interaction:201").unwrap().is_none());
    }
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let unbound = ingress::admit_mapped_slash_prompt(&db, &request())
        .unwrap()
        .record
        .unwrap();
    assert!(
        unbound.target_thread_id.is_none(),
        "no mapping means target remains unknown"
    );
}
