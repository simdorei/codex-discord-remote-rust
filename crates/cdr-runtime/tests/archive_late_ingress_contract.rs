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
async fn late_root_and_descendant_requests_are_saved_but_never_executable() {
    for (mode, target) in [("archive_gate", "thread-b"), ("descendant_gate", "child")] {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state.sqlite");
        let connection = rusqlite::Connection::open(&state).unwrap();
        connection
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        if target == "child" {
            connection.execute("INSERT INTO threads SELECT 'child','child',cwd,updated_at,rollout_path,model,reasoning_effort,tokens_used,0,0,source,thread_source FROM threads WHERE id='thread-b'", []).unwrap();
        }
        drop(connection);
        let server = Arc::new(app::start(&root, &state, mode).await);
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
        let gate = async {
            loop {
                if app::calls(&root)
                    .iter()
                    .any(|v| v["method"] == "thread/archive")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::select! {
            result = &mut archive => panic!("archive returned before gate: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(5), gate) => result.unwrap(),
        }
        let payload = json!({"version":1,"content":"늦게 도착한 요청 그대로 보존"});
        let admitted = ingress::admit(
            &db,
            &NewIngress {
                ingress_id: "message:70".into(),
                kind: IngressKind::Message,
                event_id: Some(70),
                application_id: None,
                channel_id: 99,
                owner_user_id: 20,
                source_message_id: Some(70),
                payload: payload.clone(),
                target_thread_id: Some(target.into()),
                canonical_owner: None,
                now: 3.0,
            },
        )
        .unwrap()
        .record
        .unwrap();
        let executable =
            ingress::begin_execution(&db, "message:70", "processing", Some(target), 4.0);
        std::fs::write(root.path().join("rpc.jsonl.release"), "release").unwrap();
        let result = archive.await;
        server.close().await.unwrap();
        result.unwrap();
        assert_eq!(admitted.payload, payload);
        assert_eq!(
            admitted.state, "held",
            "late admission during {mode} must not be staged"
        );
        assert!(admitted.hold_reason.contains("archive"));
        assert!(!executable.unwrap());
        assert_eq!(
            ingress::get(&db, "message:70").unwrap().unwrap().payload,
            payload
        );
        assert!(cdr_store::queue::list(&db).unwrap().is_empty());
    }
}
