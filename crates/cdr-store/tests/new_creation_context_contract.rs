use cdr_store::ingress::{self, IngressKind, NewIngress};
use serde_json::json;

#[test]
fn creation_context_is_generation_bound_write_once_and_preserved_by_result() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    ingress::admit(
        &db,
        &NewIngress {
            ingress_id: "new".into(),
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
    assert!(ingress::record_new_creation(&db, "new", 1, Some("project-a"), 10, 2.0).is_err());
    assert!(ingress::begin_thread_start(&db, "new", 1, 2.0).unwrap());
    assert!(ingress::record_new_creation(&db, "new", 2, Some("project-a"), 10, 3.0).is_err());
    assert!(ingress::record_new_creation(&db, "new", 1, Some("project-a"), 11, 3.0).is_err());
    ingress::record_new_creation(&db, "new", 1, Some("project-a"), 10, 3.0).unwrap();
    assert!(ingress::record_new_creation(&db, "new", 1, Some("project-b"), 10, 4.0).is_err());
    ingress::record_created_thread(&db, "new", 1, "target", 5.0).unwrap();
    let frozen =
        ingress::get(&db, "new").unwrap().unwrap().outcome.unwrap()["new_creation"].clone();
    ingress::record_result(
        &db,
        "new",
        &json!({"new_creation":{"cwd":"forged"},"text":"done"}),
        6.0,
    )
    .unwrap();
    let saved = ingress::get(&db, "new").unwrap().unwrap();
    assert_eq!(saved.outcome.unwrap()["new_creation"], frozen);
    assert_eq!(saved.target_thread_id.as_deref(), Some("target"));
    assert!(ingress::record_new_creation(&db, "new", 1, Some("project-b"), 10, 7.0).is_err());
}
