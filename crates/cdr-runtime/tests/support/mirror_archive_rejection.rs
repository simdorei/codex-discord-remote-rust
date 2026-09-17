use super::*;
use cdr_store::{claims, ingress};
use serde_json::json;

fn rejection(db: &std::path::Path, id: i64, outcome: &serde_json::Value) -> String {
    let choice = claims::create_busy_choice(
        db,
        claims::NewBusyChoice {
            owner_user_id: 42,
            channel_id: 31,
            target_thread_id: Some("thread-old"),
            prompt: "do not replay",
            allow_steer: false,
            now: 1.0,
            time_to_live: 10.0,
        },
    )
    .unwrap();
    let key = format!("interaction:{id}");
    let request = ingress::NewIngress {
        ingress_id: key.clone(),
        kind: ingress::IngressKind::Interaction,
        event_id: Some(id),
        application_id: Some(1),
        channel_id: 31,
        owner_user_id: 42,
        source_message_id: Some(500),
        payload: json!({"version":1}),
        target_thread_id: None,
        canonical_owner: None,
        now: 2.0,
    };
    ingress::admit_busy_interaction(db, &request, &choice, "steer").unwrap();
    ingress::acknowledge(db, &key, 3.0).unwrap();
    ingress::begin_execution(db, &key, "processing", None, 4.0).unwrap();
    ingress::record_result(db, &key, outcome, 5.0).unwrap();
    ingress::hold(db, &key, "previous runtime ended", false, 6.0).unwrap();
    // Offline fixture: the separate recovery notice has settled. This does NOT
    // confirm the original interaction's error response, which remains unknown.
    Connection::open(db)
        .unwrap()
        .execute(
            "DELETE FROM codex_delivery_outbox WHERE job_id=?",
            [format!("ingress-hold:{key}")],
        )
        .unwrap();
    key
}

fn no_dispatch() -> serde_json::Value {
    json!({"kind":"busy_control_preflight_rejected","control_dispatched":false})
}

#[tokio::test]
async fn archived_rejected_controls_are_preserved_without_blocking_mirror_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let db = temp.path().join("mirror.sqlite");
    let keys = [
        rejection(&db, 801, &no_dispatch()),
        rejection(&db, 802, &no_dispatch()),
    ];
    let before = keys
        .iter()
        .map(|key| ingress::get(&db, key).unwrap().unwrap())
        .collect::<Vec<_>>();
    // Ordinary cleanup remains conservative. Only a freshly archived path may reconcile.
    assert_eq!(
        cdr_store::room_cleanup::pending_reason(&db, 31, Some("thread-old")).unwrap(),
        Some("ingress")
    );
    let result = sync.sync(99, None).await.unwrap();
    assert_eq!(result.archived, 1);
    assert!(!remote.0.lock().unwrap().channels.contains_key(&31));
    assert!(thread_channels(&db, "thread-old").unwrap().is_none());
    for (key, original) in keys.iter().zip(before) {
        assert_eq!(ingress::get(&db, key).unwrap().unwrap(), original);
    }
    assert_eq!(
        cdr_store::room_cleanup::phase(&db, 31).unwrap().as_deref(),
        Some("deleted")
    );
}

#[tokio::test]
async fn mirror_inventory_includes_cli_and_app_server_user_roots() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, _) = fixture(&temp);
    let state = Connection::open(temp.path().join("state.sqlite")).unwrap();
    state
        .execute("UPDATE threads SET source='cli' WHERE id='thread-a'", [])
        .unwrap();
    state
        .execute(
            "UPDATE threads SET source='app-server' WHERE id='thread-b'",
            [],
        )
        .unwrap();
    let text = sync.inspect(99, None, true).await.unwrap();
    assert!(text.contains("missing_mapping | thread-b"), "{text}");
    let result = sync.sync(99, None).await.unwrap();
    assert_eq!(result.threads, 2);
    assert!(
        thread_channels(&temp.path().join("mirror.sqlite"), "thread-b")
            .unwrap()
            .is_some()
    );
}

#[path = "mirror_archive_safety.rs"]
mod safety;
