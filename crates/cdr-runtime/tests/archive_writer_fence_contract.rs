use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use cdr_store::ingress::{self, IngressKind, NewIngress};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/archive_app_server.rs"]
mod app;

#[tokio::test]
async fn archive_fences_ingress_before_waiting_for_transport_writer() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let server = Arc::new(app::start(&root, &state, "writer_gate").await);
    let db = root.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-b")).unwrap();
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        bridge,
        Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(AppServerTurnBackend::new(server.clone())),
        )),
    )
    .with_server(server.clone());
    let archive = executor.execute(CommandAction::Archive { reference: None }, 99, 20);
    tokio::pin!(archive);
    let lists = async {
        loop {
            if app::calls(&root)
                .iter()
                .filter(|v| v["method"] == "thread/list")
                .count()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::select! {
        result = &mut archive => panic!("archive before second list: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(4), lists) => result.unwrap(),
    }
    let mut writer = Box::pin(server.request(
        "test/blocked",
        json!({"padding":"x".repeat(1_048_576)}),
        Duration::from_secs(6),
        None,
    ));
    assert!(futures_util::poll!(writer.as_mut()).is_pending());
    std::fs::write(root.path().join("rpc.jsonl.list_release"), "release").unwrap();
    let fence = async {
        loop {
            if fence_present(&db) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::select! {
        result = &mut archive => panic!("archive did not wait for writer: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(4), fence) => result.unwrap(),
    }
    assert!(
        !app::calls(&root)
            .iter()
            .any(|v| v["method"] == "thread/archive")
    );
    let payload = json!({"version":1,"content":"writer 대기 중 새 요청"});
    let saved = ingress::admit(&db, &late_request(payload.clone()))
        .unwrap()
        .record
        .unwrap();
    std::fs::write(root.path().join("rpc.jsonl.writer_release"), "release").unwrap();
    let (written, archived) = tokio::join!(writer, archive);
    server.close().await.unwrap();
    written.unwrap();
    archived.unwrap();
    assert_eq!(saved.state, "held");
    assert_eq!(saved.payload, payload);
    assert!(
        !ingress::begin_execution(&db, "message:71", "processing", Some("thread-b"), 4.0).unwrap()
    );
    assert_eq!(
        app::calls(&root)
            .iter()
            .filter(|v| v["method"] == "thread/archive")
            .count(),
        1
    );
}

fn fence_present(db: &std::path::Path) -> bool {
    rusqlite::Connection::open(db)
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id='thread-b')",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn late_request(payload: serde_json::Value) -> NewIngress {
    NewIngress {
        ingress_id: "message:71".into(),
        kind: IngressKind::Message,
        event_id: Some(71),
        application_id: None,
        channel_id: 99,
        owner_user_id: 20,
        source_message_id: Some(71),
        payload,
        target_thread_id: Some("thread-b".into()),
        canonical_owner: None,
        now: 3.0,
    }
}
