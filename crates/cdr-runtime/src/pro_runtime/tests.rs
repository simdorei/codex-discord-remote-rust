use super::*;
use crate::test_support::{pro_connection as connection, pro_plugins as plugins};
use crate::{
    action_executor::ActionExecutor,
    app_backend::AppServerTurnBackend,
    bridge_state::BridgeState,
    queue_runner::QueueCoordinator,
    test_support::{http_gate, message_fixture::MessageFixture},
};
mod negative;

#[tokio::test]
async fn actual_pro_message_reaches_rich_turn_input_for_the_original_project() {
    for raw in ["!pro 확인", "!pro review 코드\n검수"] {
        let temp = tempfile::tempdir().unwrap();
        let gate = http_gate::start().await;
        let http = Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        );
        let mut fixture = MessageFixture::new(&temp, http).await;
        let project = temp.path().join("실제 프로젝트");
        std::fs::create_dir(&project).unwrap();
        rusqlite::Connection::open(temp.path().join("state.sqlite"))
            .unwrap()
            .execute(
                "UPDATE threads SET cwd=?1 WHERE id='thread-b'",
                [project.to_str().unwrap()],
            )
            .unwrap();
        let connected = connection::Connection::start().await;
        let exe = plugins::installed(temp.path());
        let pro = ProPromptRuntime::capture(
            temp.path().to_owned(),
            temp.path().join("state.sqlite"),
            exe,
            fixture.server.clone(),
            Some(connected.config.clone()),
            connected.status.clone(),
        )
        .await;
        let skill = temp
            .path()
            .join("plugins/codex-discord-remote/skills/ask-chatgpt-pro/SKILL.md");
        let queue = Arc::new(QueueCoordinator::new(
            fixture.executor.mirror_db().to_owned(),
            Arc::new(AppServerTurnBackend::new(fixture.server.clone()).with_pro_skill_path(&skill)),
        ));
        fixture.executor = ActionExecutor::new(
            temp.path().join("state.sqlite"),
            fixture.executor.mirror_db().to_owned(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            queue.clone(),
        )
        .with_server(fixture.server.clone())
        .with_prompt_preprocessor(Arc::new(pro));
        fixture.queue = queue;
        let admitted = fixture.admit(raw);
        gate.release.send(()).unwrap();
        crate::message_worker::process_admitted_gateway_message(
            admitted,
            &fixture.context(temp.path()),
        )
        .await
        .unwrap();
        gate.entered.await.unwrap();
        gate.stop.send(()).unwrap();
        let posts = gate.task.await.unwrap();
        connected.close().await;
        fixture.server.close().await.unwrap();
        let calls = crate::test_support::app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
        let starts: Vec<_> = calls
            .iter()
            .filter(|v| v["method"] == "turn/start")
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["params"]["threadId"], "thread-b");
        let input = starts[0]["params"]["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[1]["type"], "skill");
        assert_eq!(input[1]["path"], skill.to_str().unwrap());
        assert_eq!(input[2]["path"], "plugin://chrome@openai-bundled");
        let text = input[0]["text"].as_str().unwrap();
        assert!(text.contains("connector=\"Simdorei Local Project Oauth\""));
        assert!(text.contains("device_id=\"r10-device-fixture\""));
        assert!(
            text.contains(&format!("working_directory=\"{}\"", project.display())),
            "ticket used bot root instead of the original project: {text}"
        );
        assert!(text.contains(&cdr_pro::prompt::pro_conversation_scope("thread-b")));
        assert_eq!(text.contains("<pro-review>"), raw.contains("review"));
        assert!(
            posts[0]["content"]
                .as_str()
                .unwrap()
                .contains("In progress")
        );
        assert!(!calls.iter().any(|v| v["method"] == "thread/fork"));
    }
}
