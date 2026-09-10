use super::*;
use cdr_store::room_cleanup::archive;

#[test]
fn permanent_archive_fence_survives_reopen_and_saves_late_requests() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    let token = archive::begin(&db, "target", 2.0).unwrap();
    for channel in [31, 99] {
        let incoming = request(channel, channel);
        let saved = ingress::admit(&db, &incoming).unwrap().record.unwrap();
        assert_eq!(saved.state, "held");
        assert_eq!(saved.payload, incoming.payload);
    }
    archive::complete(&db, "target", &token).unwrap();
    assert!(mapping::upsert_thread(&db, "target", "project", "title", 90, 44, 3.0).is_err());
    assert!(archive::begin(&db, "target", 4.0).is_err());
}

#[test]
fn only_the_exact_executing_delete_confirmation_can_be_excluded() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    let mut command = request(31, 1);
    command.payload =
        serde_json::json!({"plan":{"Execute":{"DeleteArchiveConfirm":{"reference":"target"}}}});
    ingress::admit(&db, &command).unwrap();
    assert!(ingress::begin_execution(&db, "message:1", "processing", Some("target"), 2.0).unwrap());
    let mut confirmation = archive::Confirmation {
        message_id: 1,
        channel_id: 31,
        user_id: 99,
        reference: "target",
    };
    assert!(archive::begin_confirmed(&db, "target", 3.0, Some(&confirmation)).is_err());
    confirmation.user_id = 42;
    let token = archive::begin_confirmed(&db, "target", 3.0, Some(&confirmation)).unwrap();
    archive::complete(&db, "target", &token).unwrap();
    ingress::record_result(
        &db,
        "message:1",
        &serde_json::json!({"response":"deleted"}),
        4.0,
    )
    .unwrap();
    ingress::confirm(&db, "message:1", 5.0).unwrap();
    assert_eq!(
        ingress::get(&db, "message:1").unwrap().unwrap().state,
        "completed"
    );
}

#[test]
fn valid_confirmation_does_not_exclude_another_pending_request() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    seed(&db);
    let mut command = request(31, 1);
    command.payload =
        serde_json::json!({"plan":{"Execute":{"DeleteArchiveConfirm":{"reference":"target"}}}});
    ingress::admit(&db, &command).unwrap();
    ingress::begin_execution(&db, "message:1", "processing", Some("target"), 2.0).unwrap();
    ingress::admit(&db, &request(99, 2)).unwrap();
    let confirmation = archive::Confirmation {
        message_id: 1,
        channel_id: 31,
        user_id: 42,
        reference: "target",
    };
    assert!(
        archive::begin_confirmed(&db, "target", 3.0, Some(&confirmation))
            .unwrap_err()
            .to_string()
            .contains("ingress")
    );
}
