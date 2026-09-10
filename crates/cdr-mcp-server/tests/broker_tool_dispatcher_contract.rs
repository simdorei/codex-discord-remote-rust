use std::time::Duration;

use cdr_mcp_server::broker::{BridgeBroker, ProjectRegistration};
use cdr_mcp_server::mcp_http::{DispatchOutput, ToolCallContext, ToolDispatcher};
use cdr_mcp_server::tool_dispatcher::BrokerToolDispatcher;
use cdr_remote_protocol::ProjectOperationOutput;
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::output::CoreOutput;
use chrono::{TimeDelta, Utc};
use serde_json::{Map, json};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn public_tool_names_map_to_bridge_commands_and_outputs() {
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
    tokio::spawn(async move {
        while let Some(command) = bridge.commands.recv().await {
            let result = match command {
                GatewayCommand::ProjectSession { request_id, .. } => {
                    BridgeResult::ProjectSessionResult { request_id }
                }
                GatewayCommand::ProjectOperation {
                    request_id,
                    operation,
                    ..
                } => {
                    assert_eq!(
                        serde_json::to_value(operation).expect("serialize operation")["kind"],
                        "repo_status"
                    );
                    BridgeResult::ProjectOperationResult {
                        request_id,
                        output: ProjectOperationOutput::Core(CoreOutput::RepoStatus {
                            branch: "main".into(),
                            dirty_files: Vec::new(),
                            staged_files: Vec::new(),
                            remotes: vec!["origin".into()],
                            upstream: Some("origin/main".into()),
                            ahead: 0,
                            behind: 0,
                        }),
                    }
                }
                other => panic!("unexpected command: {other:?}"),
            };
            responder.complete(&identity, result).await;
        }
    });
    let dispatcher = BrokerToolDispatcher::new(broker, "https://simdorei.duckdns.org/mcp".into());
    let context = context();
    let selected = dispatcher
        .dispatch(
            context.clone(),
            "select_project".into(),
            object(&json!({
                "project_scope":"project-scope-1234",
                "connector_resource":"https://simdorei.duckdns.org/mcp"
            })),
        )
        .await
        .expect("select project");
    assert_eq!(structured(selected)["project_name"], "Sample");
    let status = dispatcher
        .dispatch(context, "repo_status".into(), Map::new())
        .await
        .expect("repo status");
    assert_eq!(structured(status)["branch"], "main");
}

fn context() -> ToolCallContext {
    ToolCallContext {
        session: "chat-a".into(),
        subject: "principal-a".into(),
        request_id: "request-1234567890".into(),
        cancellation: CancellationToken::new(),
    }
}

fn object(value: &serde_json::Value) -> Map<String, serde_json::Value> {
    value.as_object().expect("object").clone()
}

fn structured(output: DispatchOutput) -> serde_json::Value {
    let DispatchOutput::Structured(value) = output else {
        panic!("expected structured output")
    };
    value
}
