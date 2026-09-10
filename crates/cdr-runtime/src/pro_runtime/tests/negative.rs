use super::*;
use std::time::Duration;

#[tokio::test]
async fn actual_pro_failures_never_start_a_turn_or_choose_another_project() {
    for (case, expected) in [
        ("offline", "connection did not become ready"),
        ("stale", "plugins changed"),
        ("missing", "project directory could not be verified"),
        ("invalid-plugin", "invalid plugin source metadata"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let gate = http_gate::start().await;
        let http = Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        );
        gate.release.send(()).unwrap();
        let mut fixture = MessageFixture::new(&temp, http).await;
        let project = temp.path().join("original-project");
        std::fs::create_dir(&project).unwrap();
        rusqlite::Connection::open(temp.path().join("state.sqlite"))
            .unwrap()
            .execute(
                "UPDATE threads SET cwd=?1 WHERE id='thread-b'",
                [project.to_str().unwrap()],
            )
            .unwrap();
        let mut connected = Some(connection::Connection::start().await);
        let connection = connected.as_ref().unwrap();
        let pro = ProPromptRuntime::capture(
            temp.path().to_owned(),
            temp.path().join("state.sqlite"),
            plugins::installed(temp.path()),
            fixture.server.clone(),
            Some(connection.config.clone()),
            connection.status.clone(),
        )
        .await;
        match case {
            "offline" => connected.take().unwrap().close().await,
            "stale" => {
                // Another owned turn makes automatic quiescent refresh unavailable.
                fixture
                    .server
                    .request(
                        "test/active-turn",
                        serde_json::json!({"threadId":"other-thread","turnId":"other-turn"}),
                        Duration::from_secs(2),
                        None,
                    )
                    .await
                    .unwrap();
                std::fs::write(temp.path().join("chrome/plugin.txt"), "changed fixture").unwrap();
            }
            "missing" => std::fs::remove_dir(&project).unwrap(),
            "invalid-plugin" => {
                std::fs::write(temp.path().join("inventory.json"), "not JSON").unwrap();
            }
            _ => unreachable!(),
        }
        fixture.executor = fixture.executor.with_prompt_preprocessor(Arc::new(pro));
        let generation = fixture.server.generation();
        let admitted = fixture.admit("!pro review 확인");
        let result = crate::message_worker::process_admitted_gateway_message(
            admitted,
            &fixture.context(temp.path()),
        )
        .await;
        let error = result.unwrap_err().to_string();
        gate.stop.send(()).unwrap();
        assert!(gate.task.await.unwrap().is_empty());
        assert!(error.contains(expected), "{case}: {error}");
        assert_eq!(fixture.server.generation(), generation);
        if let Some(connection) = connected {
            connection.close().await;
        }
        fixture.server.close().await.unwrap();
        let calls = crate::test_support::app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
        assert!(!calls.iter().any(|v| matches!(
            v["method"].as_str(),
            Some("turn/start" | "thread/fork" | "thread/start")
        )));
        let ingress = cdr_store::ingress::get(fixture.executor.mirror_db(), "message:801")
            .unwrap()
            .unwrap();
        assert_eq!(ingress.target_thread_id.as_deref(), Some("thread-b"));
    }
}

#[tokio::test]
async fn ordinary_text_does_not_require_pro_plugins_or_remote_connection() {
    let temp = tempfile::tempdir().unwrap();
    let server = Arc::new(
        crate::test_support::app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl"))
            .await,
    );
    let pro = ProPromptRuntime::capture(
        temp.path().to_owned(),
        temp.path().join("absent.sqlite"),
        temp.path().join("missing-cli.exe"),
        server.clone(),
        None,
        RemoteAgentStatus::default(),
    )
    .await;
    assert_eq!(
        pro.prepare("ordinary request", "original").await.unwrap(),
        "ordinary request"
    );
    server.close().await.unwrap();
}
