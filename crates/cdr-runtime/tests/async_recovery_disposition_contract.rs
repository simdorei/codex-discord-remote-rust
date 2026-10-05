//! Read-only custody facts are not permission to repeat an old lifecycle action.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{ffi::OsString, path::PathBuf};

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod fixture;

struct Inventory {
    _temp: tempfile::TempDir,
    path: PathBuf,
}

impl Inventory {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite");
        fixture::dispatching(&path, "resident");
        fixture::pending(&path, "next", "thread-b", 1);
        Self { _temp: temp, path }
    }

    fn legacy() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy.sqlite");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.pragma_update(
            None,
            "user_version",
            cdr_store::schema::LATEST_STORE_SCHEMA_VERSION,
        )
        .unwrap();
        db.execute_batch(
            "CREATE TABLE discord_ingress_journal(
            ingress_id TEXT,kind TEXT,event_id INTEGER,channel_id INTEGER,owner_user_id INTEGER,
            source_message_id INTEGER,payload_json TEXT,state TEXT,phase TEXT,target_thread_id TEXT,
            created_at REAL,updated_at REAL,owner_kind TEXT,owner_id TEXT,
            confirmation_delivered INTEGER DEFAULT 0);",
        )
        .unwrap();
        Self { _temp: temp, path }
    }

    fn insert(&self, id: i64, payload: &str, target: Option<&str>) {
        rusqlite::Connection::open(&self.path)
            .unwrap()
            .execute(
                "INSERT INTO discord_ingress_journal
             (ingress_id,kind,event_id,channel_id,owner_user_id,source_message_id,
              payload_json,state,phase,target_thread_id,created_at,updated_at)
             VALUES (?1,'message',?2,123,456,?2,?3,'held','processing',?4,1,1)",
                rusqlite::params![format!("message:{id}"), id, payload, target],
            )
            .unwrap();
    }

    async fn read(&self) -> Value {
        let before = std::fs::read(&self.path).unwrap();
        let args = vec![
            OsString::from("inspect-async-recovery"),
            OsString::from("--database"),
            self.path.as_os_str().to_owned(),
            OsString::from("--thread-id"),
            OsString::from("thread-b"),
        ];
        let text = Box::pin(cdr_runtime::admin::run(args)).await.unwrap();
        assert_eq!(std::fs::read(&self.path).unwrap(), before);
        let report: Value = serde_json::from_str(&text).unwrap();
        for key in [
            "execution_authorized",
            "receipt_apply_authorized",
            "publication_authorized",
        ] {
            assert_eq!(report[key], false);
        }
        let rows = report["lifecycle_evidence"]["unresolved_ingress"]["rows"].clone();
        for row in rows.as_array().unwrap() {
            assert_eq!(row["intent_evidence"]["diagnostic_only"], true);
            assert_eq!(row["intent_evidence"]["replay_authorized"], false);
            assert_eq!(row["intent_evidence"]["live_route_verified"], false);
            assert_eq!(row["intent_evidence"]["effect_verified"], false);
        }
        rows
    }
}

fn bound(action: &str) -> Value {
    let command = json!({action:{"reference":null}});
    json!({"version":1,"plan":{"Execute":command},
        "lifecycle_binding":{"target":"thread-b","route":"Mapped","command":command},
        "private_text":"DO_NOT_EXPOSE_OR_REPLAY"})
}

#[tokio::test]
async fn missing_and_explicit_null_bindings_are_distinguished_without_inventing_targets() {
    let f = Inventory::new();
    let mut absent = bound("Stop");
    absent.as_object_mut().unwrap().remove("lifecycle_binding");
    let mut null = bound("Archive");
    null["lifecycle_binding"] = Value::Null;
    f.insert(11, &absent.to_string(), Some("thread-b"));
    f.insert(12, &null.to_string(), Some("thread-b"));
    let rows = f.read().await;
    for (index, status) in ["missing", "null"].into_iter().enumerate() {
        assert_eq!(rows[index]["intent_evidence"]["binding_presence"], status);
        assert_eq!(
            rows[index]["intent_evidence"]["stored_custody_consistent"],
            false
        );
        assert_eq!(
            rows[index]["intent_evidence"]["disposition"],
            "preserve_require_fresh_bound_intent"
        );
    }
}

#[tokio::test]
async fn internally_consistent_held_controls_still_require_original_effect_reconciliation() {
    let f = Inventory::new();
    for (id, action) in [(21, "Stop"), (22, "Archive")] {
        f.insert(id, &bound(action).to_string(), Some("thread-b"));
    }
    let rows = f.read().await;
    for row in rows.as_array().unwrap() {
        let evidence = &row["intent_evidence"];
        assert_eq!(evidence["stored_custody_consistent"], true);
        assert_eq!(evidence["route"], "Mapped");
        assert_eq!(
            evidence["disposition"],
            "preserve_reconcile_original_effect"
        );
        assert_eq!(row["state"], "held");
        assert_eq!(row["phase"], "processing");
    }
    assert!(!rows.to_string().contains("DO_NOT_EXPOSE_OR_REPLAY"));
}

#[tokio::test]
async fn target_command_version_and_route_mismatches_never_appear_consistent() {
    let f = Inventory::new();
    let changes = [
        ("/lifecycle_binding/target", json!("other")),
        (
            "/lifecycle_binding/command",
            json!({"Archive":{"reference":null}}),
        ),
        ("/lifecycle_binding/route", json!("FutureRoute")),
        ("/lifecycle_binding/route", json!("Explicit")),
        ("/lifecycle_binding/command/Stop/reference", json!(17)),
        ("/version", json!(2)),
        ("/plan/Execute", json!({"Stop":{}})),
    ];
    for (index, (pointer, value)) in changes.into_iter().enumerate() {
        let mut payload = bound("Stop");
        *payload.pointer_mut(pointer).unwrap() = value;
        f.insert(
            30 + i64::try_from(index).unwrap(),
            &payload.to_string(),
            Some("thread-b"),
        );
    }
    let rows = f.read().await;
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|row| row["intent_evidence"]["stored_custody_consistent"] == false)
    );
}

#[tokio::test]
async fn source_message_owner_and_unbound_target_are_not_repaired_from_the_current_route() {
    let f = Inventory::new();
    for id in 50..55 {
        f.insert(id, &bound("Stop").to_string(), Some("thread-b"));
    }
    let db = rusqlite::Connection::open(&f.path).unwrap();
    db.execute_batch(
        "UPDATE discord_ingress_journal SET source_message_id=999 WHERE event_id=50;
        UPDATE discord_ingress_journal SET owner_user_id=0 WHERE event_id=51;
        UPDATE discord_ingress_journal SET kind='action' WHERE event_id=52;
        UPDATE discord_ingress_journal SET target_thread_id=NULL WHERE event_id=53;
        UPDATE discord_ingress_journal SET ingress_id='message:wrong' WHERE event_id=54;",
    )
    .unwrap();
    drop(db);
    let rows = f.read().await;
    assert_eq!(rows.as_array().unwrap().len(), 5);
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|row| row["intent_evidence"]["stored_custody_consistent"] == false)
    );
}

#[tokio::test]
async fn malformed_oversized_and_opaque_payloads_remain_incomplete_and_private() {
    // Current insertion guards reject malformed JSON. A readonly legacy import
    // can still contain it; do not disable any current production guard.
    let f = Inventory::legacy();
    let malformed = "{DO_NOT_EXPOSE_OR_REPLAY";
    let mut oversized = bound("Archive");
    oversized["private_text"] = "x".repeat(131_073).into();
    for (id, payload) in [
        (61, malformed.to_owned()),
        (62, oversized.to_string()),
        (63, "[]".into()),
    ] {
        f.insert(id, &payload, Some("thread-b"));
    }
    let rows = f.read().await;
    assert_eq!(rows[0]["intent_evidence"]["payload_status"], "invalid");
    assert_eq!(rows[0]["intent_evidence"]["binding_presence"], "unknown");
    assert_eq!(rows[1]["intent_evidence"]["payload_status"], "unavailable");
    assert_eq!(rows[1]["intent_evidence"]["binding_presence"], "unknown");
    assert_eq!(rows[1]["details_oversized"], true);
    assert_eq!(rows[2]["intent_evidence"]["payload_status"], "invalid");
    assert_eq!(
        rows[0]["details_sha256"],
        hex::encode(Sha256::digest(malformed.as_bytes()))
    );
    assert!(!rows.to_string().contains("DO_NOT_EXPOSE_OR_REPLAY"));
}

#[tokio::test]
async fn extra_fields_and_noncontrol_plans_are_not_lifecycle_clearance() {
    let f = Inventory::new();
    let mut extra = bound("Stop");
    extra["lifecycle_binding"]["future_authority"] = true.into();
    let mut noncontrol = bound("Stop");
    noncontrol["plan"]["Execute"] = json!({"Ask":{"prompt":"DO_NOT_EXPOSE_OR_REPLAY"}});
    for (id, payload) in [(71, extra), (72, noncontrol)] {
        f.insert(id, &payload.to_string(), Some("thread-b"));
    }
    let rows = f.read().await;
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|row| row["intent_evidence"]["stored_custody_consistent"] == false)
    );
    assert!(!rows.to_string().contains("DO_NOT_EXPOSE_OR_REPLAY"));
}

#[tokio::test]
async fn explicit_and_selected_routes_describe_saved_facts_not_live_selection_permission() {
    let f = Inventory::new();
    let mut explicit = bound("Archive");
    explicit["plan"]["Execute"]["Archive"]["reference"] = "private-alias".into();
    explicit["lifecycle_binding"]["command"] = explicit["plan"]["Execute"].clone();
    explicit["lifecycle_binding"]["route"] = "Explicit".into();
    let mut selected = bound("Stop");
    selected["lifecycle_binding"]["route"] = "Selected".into();
    for (id, payload) in [(81, explicit), (82, selected)] {
        f.insert(id, &payload.to_string(), Some("thread-b"));
    }
    let rows = f.read().await;
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|row| row["intent_evidence"]["stored_custody_consistent"] == true)
    );
    assert!(!rows.to_string().contains("private-alias"));
}
