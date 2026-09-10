use std::time::Duration;

use cdr_mcp_server::broker::{BridgeBroker, ProjectRegistration};
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use chrono::{TimeDelta, Utc};

#[tokio::test]
async fn project_selection_and_request_round_trip_through_one_bridge() {
    let broker = BridgeBroker::new(Duration::from_secs(2));
    let mut bridge = broker.attach("windows-a").await;
    let identity = bridge.identity().clone();
    broker
        .upsert(
            &identity,
            ProjectRegistration {
                project_scope: "project-scope-1234".into(),
                binding_id: "binding-12345678".into(),
                thread_id: "thread-a".into(),
                project_name: "Sample".into(),
                expires_at: Utc::now() + TimeDelta::minutes(5),
            },
        )
        .await
        .expect("register project");
    let responder = broker.clone();
    let responder_identity = identity.clone();
    tokio::spawn(async move {
        while let Some(command) = bridge.commands.recv().await {
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
                    output: cdr_remote_protocol::message::ProjectInfoOutput {
                        root: "C:\\work\\sample".into(),
                        thread_id,
                    },
                },
                other => panic!("unexpected command: {other:?}"),
            };
            responder.complete(&responder_identity, result).await;
        }
    });

    let selected = broker
        .select_project("chat-a", "principal-a", "project-scope-1234")
        .await
        .expect("select project");
    assert_eq!(selected.project_name, "Sample");
    let info = broker
        .project_info("chat-a", "principal-a")
        .await
        .expect("request project info");
    assert_eq!(info.root, "C:\\work\\sample");
}
