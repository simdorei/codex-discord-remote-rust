use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    interaction_worker::run_interaction_worker, queue_runner::QueueCoordinator,
};
use cdr_store::mapping;
use std::{sync::Arc, time::Duration};
#[path = "support/approval_app_server.rs"]
mod app;
#[path = "support/approval_http.rs"]
mod http;
#[path = "support/approval_owner.rs"]
mod owner;
#[path = "support/mapped_slash.rs"]
mod slash;

#[tokio::test]
async fn slash_approval_delivers_bound_prompt_and_finishes_original_interaction() {
    run_approval(None).await;
}

#[tokio::test]
async fn slash_approval_keeps_valid_ui_with_app_only_input_in_either_order() {
    run_approval(Some(true)).await;
    run_approval(Some(false)).await;
}

async fn run_approval(secret_first: Option<bool>) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let server = Arc::new(app::start(&temp, &temp.path().join("rpc.jsonl")).await);
    mapping::upsert_thread(&db, "thread-b", "p", "b", 10, 42, 1.0).unwrap();
    owner::running(&db, server.generation());
    if secret_first == Some(true) {
        add_secret(&server).await;
    }
    server
        .request(
            "test/pending",
            serde_json::json!({}),
            Duration::from_secs(2),
            None,
        )
        .await
        .unwrap();
    if secret_first == Some(false) {
        add_secret(&server).await;
    }
    let before = server.pending_server_requests(None).await.unwrap();
    let executor = Arc::new(
        ActionExecutor::new(
            temp.path().join("state.sqlite"),
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            Arc::new(QueueCoordinator::new(
                db.clone(),
                Arc::new(AppServerTurnBackend::new(server.clone())),
            )),
        )
        .with_server(server.clone()),
    );
    let fixture = http::start().await;
    let client = Arc::new(
        twilight_http::Client::builder()
            .proxy(fixture.address, true)
            .ratelimiter(None)
            .build(),
    );
    let work = slash::stage(&db, "approval").await;
    let (send, recv) = tokio::sync::mpsc::channel(1);
    send.send(work).await.unwrap();
    drop(send);
    tokio::time::timeout(
        Duration::from_secs(3),
        run_interaction_worker(recv, executor, server.clone(), client),
    )
    .await
    .unwrap();
    fixture.stop.send(()).unwrap();
    let traffic = fixture.task.await.unwrap();
    assert_eq!(traffic.len(), if secret_first.is_some() { 3 } else { 2 });
    let approval = traffic
        .iter()
        .find(|(post, body)| {
            *post
                && body["content"]
                    .as_str()
                    .unwrap()
                    .contains("Approval required")
        })
        .unwrap();
    let summary = traffic.iter().find(|(post, _)| !post).unwrap();
    if secret_first.is_some() {
        assert!(traffic.iter().any(|(_, body)| {
            body["content"]
                .as_str()
                .unwrap()
                .contains("secret input requires the Codex app")
        }));
        assert!(
            !traffic
                .iter()
                .any(|(_, body)| body["content"].as_str().unwrap().contains("DO NOT DISPLAY"))
        );
    }
    assert_eq!(
        approval.1["components"][0]["components"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(summary.1["content"].as_str().unwrap().contains(&format!(
        "Existing Codex approval/input requests: {}",
        before.len()
    )));
    assert_eq!(
        cdr_store::ingress::get(&db, "interaction:101")
            .unwrap()
            .unwrap()
            .state,
        "completed"
    );
    assert_eq!(server.pending_server_requests(None).await.unwrap(), before);
    server.close().await.unwrap();
    assert_no_response_or_control(&temp.path().join("rpc.jsonl"));
}

fn assert_no_response_or_control(log: &std::path::Path) {
    let frames = std::fs::read_to_string(log).unwrap();
    assert!(!frames.lines().any(|line| {
        let frame: serde_json::Value = serde_json::from_str(line).unwrap();
        (frame.get("result").is_some()
            && matches!(frame["id"].as_str(), Some("approval-1" | "secret-2")))
            || matches!(
                frame["method"].as_str(),
                Some("turn/start" | "thread/resume" | "thread/fork")
            )
    }));
}

async fn add_secret(server: &cdr_app_server::ResidentAppServer) {
    server.request("test/pending",serde_json::json!({"requestId":"secret-2","method":"item/tool/requestUserInput","questions":[{"id":"private","question":"DO NOT DISPLAY","isSecret":true,"options":null}]}),Duration::from_secs(2),None).await.unwrap();
}
