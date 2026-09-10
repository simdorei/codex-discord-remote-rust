use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    mapping, room_cleanup,
};
#[path = "support/archive_cleanup_contract.rs"]
mod archive_cleanup;
use std::{
    path::Path,
    sync::{Arc, Barrier},
};

fn seed(path: &Path) {
    mapping::upsert_thread(path, "target", "project", "title", 90, 31, 1.0).unwrap();
}
fn request(channel: i64, event: i64) -> NewIngress {
    NewIngress {
        ingress_id: format!("message:{event}"),
        kind: IngressKind::Message,
        event_id: Some(event),
        application_id: None,
        channel_id: channel,
        owner_user_id: 42,
        source_message_id: Some(event),
        payload: serde_json::json!({"content":"원문 보존"}),
        target_thread_id: Some("target".into()),
        canonical_owner: None,
        now: 2.0,
    }
}

#[test]
fn earlier_ingress_on_another_channel_protects_the_same_target() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    ingress::admit(&db, &request(99, 1)).unwrap();
    assert!(
        room_cleanup::begin(&db, 31, Some("target"), 3.0)
            .unwrap_err()
            .to_string()
            .contains("ingress")
    );
    assert_eq!(room_cleanup::phase(&db, 31).unwrap(), None);
    assert_eq!(
        ingress::get(&db, "message:1").unwrap().unwrap().state,
        "staged"
    );
}

#[test]
fn late_ingress_is_saved_on_both_original_channel_and_target_routes() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    room_cleanup::begin(&db, 31, Some("target"), 1.0).unwrap();
    for channel in [31, 99] {
        let original = request(channel, channel);
        let saved = ingress::admit(&db, &original).unwrap().record.unwrap();
        assert_eq!(saved.state, "held");
        assert_eq!(saved.payload, original.payload);
        assert!(
            !ingress::begin_execution(&db, &original.ingress_id, "processing", Some("target"), 4.0)
                .unwrap()
        );
    }
    assert_eq!(
        room_cleanup::phase(&db, 31).unwrap().as_deref(),
        Some("deleting")
    );
    assert!(room_cleanup::begin(&db, 31, Some("target"), 99_999.0).is_err());
}

#[test]
fn tombstone_blocks_old_room_but_not_an_explicit_new_mapping_after_completion() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    let token = room_cleanup::begin(&db, 31, Some("target"), 1.0).unwrap();
    assert!(mapping::upsert_thread(&db, "target", "project", "title", 90, 32, 2.0).is_err());
    assert!(room_cleanup::complete(&db, 31, "wrong-token").is_err());
    room_cleanup::complete(&db, 31, &token).unwrap();
    mapping::upsert_thread(&db, "target", "project", "title", 90, 32, 3.0).unwrap();
    assert_eq!(
        ingress::admit(&db, &request(31, 1))
            .unwrap()
            .record
            .unwrap()
            .state,
        "held"
    );
    assert_eq!(
        ingress::admit(&db, &request(32, 2))
            .unwrap()
            .record
            .unwrap()
            .state,
        "staged"
    );
}

#[test]
fn cleanup_rejects_delivery_and_mapping_writes_before_they_can_start() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    let token = room_cleanup::begin(&db, 31, Some("target"), 1.0).unwrap();
    for key in ["[31,\"final\",\"job\",0]", "legacy-key", "{}"] {
        assert!(cdr_store::delivery_receipt::begin(&db, key, "hash").is_err());
    }
    assert!(mapping::upsert_thread(&db, "other", "project", "title", 90, 31, 2.0).is_err());
    assert!(room_cleanup::release_rejected(&db, 31, "wrong").is_err());
    room_cleanup::release_rejected(&db, 31, &token).unwrap();
    assert_eq!(room_cleanup::phase(&db, 31).unwrap(), None);
    assert_eq!(
        ingress::admit(&db, &request(31, 4))
            .unwrap()
            .record
            .unwrap()
            .state,
        "staged"
    );
}

#[test]
fn concurrent_admission_and_cleanup_have_a_single_atomic_boundary() {
    for _ in 0..8 {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        seed(&db);
        let gate = Arc::new(Barrier::new(2));
        let gate_copy = gate.clone();
        let db_copy = db.clone();
        let writer = std::thread::spawn(move || {
            gate_copy.wait();
            ingress::admit(&db_copy, &request(31, 1))
                .unwrap()
                .record
                .unwrap()
        });
        gate.wait();
        let close = room_cleanup::begin(&db, 31, Some("target"), 3.0);
        let saved = writer.join().unwrap();
        if close.is_ok() {
            assert_eq!(saved.state, "held");
        } else {
            assert_eq!(saved.state, "staged");
            assert_eq!(room_cleanup::phase(&db, 31).unwrap(), None);
        }
        assert_eq!(saved.payload["content"], "원문 보존");
    }
}
