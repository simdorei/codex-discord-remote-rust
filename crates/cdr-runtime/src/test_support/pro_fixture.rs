use crate::test_support::{pro_connection as connection, pro_plugins as plugins};
use crate::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    pro_runtime::ProPromptRuntime, queue_runner::QueueCoordinator,
    test_support::message_fixture::MessageFixture,
};
use std::sync::Arc;

pub(crate) async fn configure(
    temp: &tempfile::TempDir,
    fixture: &mut MessageFixture,
    case: &str,
) -> Option<connection::Connection> {
    let project = temp.path().join("original-project");
    std::fs::create_dir(&project).unwrap();
    rusqlite::Connection::open(temp.path().join("state.sqlite"))
        .unwrap()
        .execute(
            "UPDATE threads SET cwd=?1 WHERE id='thread-b'",
            [project.to_str().unwrap()],
        )
        .unwrap();
    let connected = connection::Connection::start().await;
    let pro = ProPromptRuntime::capture(
        temp.path().into(),
        temp.path().join("state.sqlite"),
        plugins::installed(temp.path()),
        fixture.server.clone(),
        Some(connected.config.clone()),
        connected.status.clone(),
    )
    .await;
    let queue = Arc::new(QueueCoordinator::new(
        fixture.executor.mirror_db().into(),
        Arc::new(
            AppServerTurnBackend::new(fixture.server.clone()).with_pro_skill_path(
                temp.path()
                    .join("plugins/codex-discord-remote/skills/ask-chatgpt-pro/SKILL.md"),
            ),
        ),
    ));
    fixture.executor = ActionExecutor::new(
        temp.path().join("state.sqlite"),
        fixture.executor.mirror_db().into(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        queue.clone(),
    )
    .with_server(fixture.server.clone())
    .with_prompt_preprocessor(Arc::new(pro));
    fixture.queue = queue;
    if case == "invalid-plugin" {
        std::fs::write(temp.path().join("inventory.json"), "not JSON").unwrap();
    }
    if case == "offline" {
        connected.close().await;
        None
    } else {
        Some(connected)
    }
}
