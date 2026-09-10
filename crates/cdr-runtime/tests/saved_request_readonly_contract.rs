use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use cdr_store::ingress::{IngressKind, NewIngress, admit};
use std::sync::Arc;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn inspecting_a_missing_store_does_not_create_or_initialize_it() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("absent.sqlite");
    let executor = target::executor(
        &root,
        db.clone(),
        Arc::new(BridgeState::new(root.path().join("bridge.json"))),
        Arc::new(target::FakeBackend::default()),
    );
    assert!(!db.exists());
    assert!(
        executor
            .execute(
                CommandAction::SavedRequest {
                    request_id: "absent".into()
                },
                42,
                3
            )
            .await
            .is_err()
    );
    assert!(
        !db.exists(),
        "read-only inspection initialized an absent store"
    );
}

#[tokio::test]
async fn unauthorized_malformed_payload_is_indistinguishable_from_missing_request() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("mirror.sqlite");
    admit(
        &db,
        &NewIngress {
            ingress_id: "action:private".into(),
            kind: IngressKind::Action,
            event_id: None,
            application_id: None,
            channel_id: 42,
            owner_user_id: 3,
            source_message_id: None,
            payload: serde_json::json!({"prompt":"original"}),
            target_thread_id: Some("thread-b".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    rusqlite::Connection::open(&db).unwrap().execute("UPDATE discord_ingress_journal SET payload_json='malformed' WHERE ingress_id='action:private'", []).unwrap();
    let executor = target::executor(
        &root,
        db.clone(),
        Arc::new(BridgeState::new(root.path().join("bridge.json"))),
        Arc::new(target::FakeBackend::default()),
    );
    let read = |id: &str| CommandAction::SavedRequest {
        request_id: id.into(),
    };
    let missing = executor
        .execute(read("missing"), 42, 4)
        .await
        .unwrap_err()
        .to_string();
    let other_user = executor
        .execute(read("action:private"), 42, 4)
        .await
        .unwrap_err()
        .to_string();
    let other_room = executor
        .execute(read("action:private"), 43, 3)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(other_user, missing);
    assert_eq!(other_room, missing);
    assert!(
        executor
            .execute(read("action:private"), 42, 3)
            .await
            .is_err(),
        "original owner sees real corruption failure"
    );
    let raw: String = rusqlite::Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT payload_json FROM discord_ingress_journal WHERE ingress_id='action:private'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(raw, "malformed");
}

#[tokio::test]
async fn unsupported_store_versions_are_reported_without_migration() {
    for version in [0, 999] {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("mirror.sqlite");
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        let executor = target::executor(
            &root,
            db.clone(),
            Arc::new(BridgeState::new(root.path().join("bridge.json"))),
            Arc::new(target::FakeBackend::default()),
        );
        let error = executor
            .execute(
                CommandAction::SavedRequest {
                    request_id: "absent".into(),
                },
                42,
                3,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains(&version.to_string()), "{error}");
        if version == 0 {
            assert!(
                !error.to_string().contains("newer"),
                "older schema mislabeled: {error}"
            );
        }
        let actual: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(actual, version);
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0, "inspection migrated the store");
        assert!(!root.path().join(".codex-discord-backups").exists());
    }
}
