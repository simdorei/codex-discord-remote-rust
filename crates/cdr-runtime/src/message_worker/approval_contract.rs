use super::*;
use crate::test_support::approval_app_fixture as app;

#[tokio::test]
async fn prefix_approval_reaches_real_message_worker_and_delivers_existing_buttons() {
    run_approval(None).await;
}

#[tokio::test]
async fn prefix_approval_keeps_normal_buttons_when_secret_input_is_also_pending() {
    run_approval(Some(true)).await;
    run_approval(Some(false)).await;
}

async fn run_approval(secret_first: Option<bool>) {
    let temp = tempfile::tempdir().unwrap();
    let server = Arc::new(app::start(&temp, &temp.path().join("rpc.jsonl")).await);
    let gate = crate::test_support::http_gate::start().await;
    let http = Arc::new(
        Client::builder()
            .proxy(gate.address, true)
            .ratelimiter(None)
            .build(),
    );
    let fixture = crate::test_support::message_fixture::MessageFixture::with_server(
        &temp,
        http,
        server.clone(),
    );
    let db = fixture.executor.mirror_db();
    crate::test_support::approval_owner::running(db, server.generation());
    if secret_first == Some(true) {
        add_secret(&server).await;
    }
    server
        .request(
            "test/pending",
            serde_json::json!({}),
            std::time::Duration::from_secs(2),
            None,
        )
        .await
        .unwrap();
    if secret_first == Some(false) {
        add_secret(&server).await;
    }
    let before = server.pending_server_requests(None).await.unwrap();
    gate.release.send(()).unwrap();
    process_admitted_gateway_message(fixture.admit("!approval"), &fixture.context(temp.path()))
        .await
        .unwrap();
    gate.entered.await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), if secret_first.is_some() { 2 } else { 1 });
    let approval = posts
        .iter()
        .find(|post| {
            post["content"]
                .as_str()
                .unwrap()
                .contains("Approval required")
        })
        .unwrap();
    if secret_first.is_some() {
        assert!(posts.iter().any(|post| {
            post["content"]
                .as_str()
                .unwrap()
                .contains("secret input requires the Codex app")
        }));
        assert!(
            !posts
                .iter()
                .any(|post| post["content"].as_str().unwrap().contains("DO NOT DISPLAY"))
        );
    }
    assert!(
        approval["content"]
            .as_str()
            .unwrap()
            .contains("Approval required")
    );
    assert_eq!(
        approval["components"][0]["components"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(server.pending_server_requests(None).await.unwrap(), before);
    assert_eq!(
        cdr_store::ingress::get(db, "message:801")
            .unwrap()
            .unwrap()
            .state,
        "completed"
    );
    server.close().await.unwrap();
}

async fn add_secret(server: &cdr_app_server::ResidentAppServer) {
    server.request("test/pending", serde_json::json!({"requestId":"secret-2","method":"item/tool/requestUserInput","questions":[{"id":"private","question":"DO NOT DISPLAY","isSecret":true,"options":null}]}),std::time::Duration::from_secs(2),None).await.unwrap();
}
