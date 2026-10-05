use cdr_store::{
    ingress::{
        self, IngressKind, NewIngress,
        stop::{StopScope, accept_nonrunning, revision},
    },
    queue::{self, NewQueueJob},
};
use serde_json::json;
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        queue::enqueue(
            &db,
            NewQueueJob {
                job_id: "original",
                target_thread_id: "target-a",
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(101),
                app_server_generation: 7,
                prompt: "original",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        Self { _temp: temp, db }
    }
    fn request(event: i64, field: &str) -> NewIngress {
        let mut payload = json!({"version":1,"plan":{"Execute":{"Settings":{"reference":"target-a","model":"model-b","effort":null,"speed":null}}}});
        payload[field] =
            json!({"target":"target-a","route":"Explicit","command":{"fixture":"bound"}});
        NewIngress {
            ingress_id: format!("message:{event}"),
            kind: IngressKind::Message,
            event_id: Some(event),
            application_id: None,
            channel_id: 99,
            owner_user_id: 20,
            source_message_id: Some(event),
            payload,
            target_thread_id: Some("target-a".into()),
            canonical_owner: None,
            now: 1.0,
        }
    }
    fn stop(&self) {
        accept_nonrunning(&self.db,StopScope{target:"target-a",channel:99,owner:20},
            &json!({"target":"target-a","route":"Explicit","command":{"Stop":{"reference":"target-a"}}}),
            None,||Ok(())).unwrap().unwrap();
    }
}

#[test]
fn bound_settings_and_lifecycle_origin_survives_duplicate_admission_after_stop() {
    for field in ["settings_binding", "lifecycle_binding"] {
        let f = Fixture::new();
        let original = Fixture::request(901, field);
        let first = ingress::admit(&f.db, &original).unwrap().record.unwrap();
        assert_eq!(
            first.payload["stop_origin"],
            json!({"target":"target-a","stopRevision":0})
        );
        assert_eq!(first.payload[field], original.payload[field]);
        f.stop();
        let mut duplicate = original.clone();
        duplicate.payload["stop_origin"] = json!({"target":"target-a","stopRevision":1});
        let again = ingress::admit(&f.db, &duplicate).unwrap();
        assert!(!again.created);
        assert_eq!(again.record.unwrap(), first);
    }
}

#[test]
fn distinct_post_stop_ingress_is_fresh_without_releasing_the_old_original() {
    let f = Fixture::new();
    let a = ingress::admit(&f.db, &Fixture::request(901, "settings_binding"))
        .unwrap()
        .record
        .unwrap();
    f.stop();
    let b = ingress::admit(&f.db, &Fixture::request(902, "settings_binding"))
        .unwrap()
        .record
        .unwrap();
    let db = rusqlite::Connection::open(&f.db).unwrap();
    assert!(revision::validate_in(&db, Some("target-a"), a.payload.get("stop_origin")).is_err());
    assert!(revision::validate_in(&db, Some("target-a"), b.payload.get("stop_origin")).is_ok());
    assert_eq!(b.payload["stop_origin"]["stopRevision"], 1);
    assert!(
        cdr_store::execution_hold::reason(&f.db, "original")
            .unwrap()
            .is_some()
    );
}

#[test]
fn supplied_origin_is_not_trusted_as_admission_evidence() {
    let f = Fixture::new();
    let mut request = Fixture::request(901, "settings_binding");
    request.payload["stop_origin"] = json!({"target":"target-a","stopRevision":999_999});
    let record = ingress::admit(&f.db, &request).unwrap().record.unwrap();
    assert_eq!(
        record.payload["stop_origin"],
        json!({"target":"target-a","stopRevision":0})
    );
}

#[test]
fn unbound_original_envelopes_are_not_rewritten() {
    let f = Fixture::new();
    let mut request = Fixture::request(901, "settings_binding");
    request.payload = json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"keep original"}}}});
    let record = ingress::admit(&f.db, &request).unwrap().record.unwrap();
    assert_eq!(record.payload, request.payload);
}

#[test]
fn null_binding_is_a_read_only_unbound_envelope() {
    let f = Fixture::new();
    let mut request = Fixture::request(902, "settings_binding");
    request.payload = json!({"version":1,"settings_binding":null,"command":"settings"});
    request.target_thread_id = None;
    let saved = ingress::admit(&f.db, &request).unwrap().record.unwrap();
    assert_eq!(saved.payload, request.payload);
}

#[test]
fn failed_admission_does_not_commit_a_processed_message_or_advance_stop() {
    let f = Fixture::new();
    let db = rusqlite::Connection::open(&f.db).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_origin BEFORE INSERT ON discord_ingress_journal
        BEGIN SELECT RAISE(ABORT,'injected admission failure'); END;",
    )
    .unwrap();
    assert!(ingress::admit(&f.db, &Fixture::request(901, "settings_binding")).is_err());
    assert!(ingress::get(&f.db, "message:901").unwrap().is_none());
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM discord_processed_messages WHERE message_id=901",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        revision::capture(&f.db, Some("target-a")).unwrap()["stopRevision"],
        0
    );
}

#[test]
fn missing_legacy_or_corrupt_origin_is_never_replaced_by_the_current_clock() {
    let f = Fixture::new();
    let original = ingress::admit(&f.db, &Fixture::request(901, "settings_binding"))
        .unwrap()
        .record
        .unwrap();
    f.stop();
    let mut legacy = original.clone();
    legacy
        .payload
        .as_object_mut()
        .unwrap()
        .remove("stop_origin");
    let frozen = revision::origin_for_ingress(&legacy).unwrap();
    assert!(frozen.is_none());
    assert!(
        revision::validate_in(
            &rusqlite::Connection::open(&f.db).unwrap(),
            Some("target-a"),
            frozen.as_ref()
        )
        .is_err()
    );
    for value in [
        json!(null),
        json!({}),
        json!({"target":"target-b","stopRevision":0}),
        json!({"target":"target-a","stopRevision":-1}),
        json!({"target":"target-a","stopRevision":"0"}),
    ] {
        let mut corrupt = original.clone();
        corrupt.payload["stop_origin"] = value;
        assert!(revision::origin_for_ingress(&corrupt).is_err());
    }
}
