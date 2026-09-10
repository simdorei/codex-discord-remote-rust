use std::time::Duration;

use cdr_mcp_server::broker::{BridgeBroker, ProjectRegistration};
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand, ProjectInfoOutput};
use chrono::{TimeDelta, Utc};

#[tokio::test]
async fn matching_project_registration_restores_the_chat_route_after_reconnect() {
    let broker = BridgeBroker::new(Duration::from_secs(2));
    let project = ProjectRegistration {
        project_scope: "project-scope-1234".into(),
        binding_id: "binding-12345678".into(),
        thread_id: "thread-a".into(),
        project_name: "Sample".into(),
        expires_at: Utc::now() + TimeDelta::minutes(5),
    };
    let mut first = broker.attach("windows-a").await;
    let first_identity = first.identity().clone();
    broker
        .upsert(&first_identity, project.clone())
        .await
        .expect("first registration");
    let first_broker = broker.clone();
    let first_result_identity = first_identity.clone();
    tokio::spawn(async move {
        let command = first.commands.recv().await.expect("selection command");
        first_broker
            .complete(
                &first_result_identity,
                BridgeResult::ProjectSessionResult {
                    request_id: command.request_id().into(),
                },
            )
            .await;
    });
    broker
        .select_project("chat-a", "principal-a", &project.project_scope)
        .await
        .expect("initial selection");
    broker.detach(&first_identity).await;

    let mut second = broker.attach("windows-a").await;
    let second_identity = second.identity().clone();
    broker
        .upsert(&second_identity, project.clone())
        .await
        .expect("replacement registration");
    let second_broker = broker.clone();
    let second_result_identity = second_identity.clone();
    tokio::spawn(async move {
        while let Some(command) = second.commands.recv().await {
            let result = match command {
                GatewayCommand::ProjectSession { request_id, .. } => {
                    BridgeResult::ProjectSessionResult { request_id }
                }
                GatewayCommand::ProjectInfo {
                    request_id,
                    thread_id,
                    ..
                } => BridgeResult::ProjectInfoResult {
                    request_id,
                    output: ProjectInfoOutput {
                        root: "C:\\work\\sample".into(),
                        thread_id,
                    },
                },
                other => panic!("unexpected command: {other:?}"),
            };
            second_broker
                .complete(&second_result_identity, result)
                .await;
        }
    });
    assert!(
        broker
            .resume_project(&second_identity, &project)
            .await
            .expect("resume route")
    );
    let info = broker
        .project_info("chat-a", "principal-a")
        .await
        .expect("restored route works");
    assert_eq!(info.thread_id, "thread-a");
}
