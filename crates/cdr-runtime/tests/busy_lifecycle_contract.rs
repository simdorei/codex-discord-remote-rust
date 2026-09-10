use cdr_app_server::requests::AppRequest;
use cdr_runtime::{
    action_executor::{ActionExecutor, ActionUi},
    app_backend::AppServerTurnBackend,
    bridge_state::BridgeState,
    command_plan::CommandAction,
    queue_runner::QueueCoordinator,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/action_app_server.rs"]
mod support;

#[tokio::test]
async fn steer_now_label_uses_the_same_lifecycle_readiness_as_click_validation() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(support::start_fake_server(&temp, &log).await);
    server
        .execute(
            AppRequest {
                method: "test/active-turn",
                params: json!({"threadId":"thread-a","turnId":"current"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let executor = ActionExecutor::new(
        temp.path().join("state.sqlite"),
        db.clone(),
        bridge,
        Arc::new(QueueCoordinator::new(
            db,
            Arc::new(AppServerTurnBackend::new(server.clone())),
        )),
    )
    .with_server(server.clone());
    for ready in [true, false] {
        if !ready {
            assert!(!server.force_restart_if_quiescent().await.unwrap());
        }
        let result = executor
            .execute(
                CommandAction::Ask {
                    prompt: "additional direction".into(),
                },
                10,
                20,
            )
            .await
            .unwrap();
        let ActionUi::Busy {
            allow_steer,
            choice_id,
        } = result.ui.unwrap()
        else {
            panic!("expected busy controls")
        };
        assert_eq!(
            allow_steer, ready,
            "Steer now must not be shown while the resident refuses controls"
        );
        let bound =
            cdr_store::control_binding::resolve(executor.mirror_db(), &choice_id, "thread-a")
                .unwrap();
        assert_eq!(
            bound.as_deref(),
            Some("current"),
            "original turn binding must remain immutable even while temporarily not ready"
        );
        assert_eq!(
            executor
                .verified_control_turn(10, "thread-a", bound.as_deref())
                .await
                .is_ok(),
            ready
        );
    }
    server.close().await.unwrap();
    assert!(!support::rpc_log(&log).iter().any(|v| matches!(
        v["method"].as_str(),
        Some("thread/fork" | "thread/resume" | "turn/start" | "turn/steer")
    )));
}
