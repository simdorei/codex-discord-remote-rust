use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::output::{ComputerOutput, ProjectOperationOutput};
use cdr_remote_protocol::request::{ComputerRequest, ProjectOperation};
use chrono::{TimeDelta, Utc};

const SESSION_A: &str = "session-aaaaaaaa";
const SESSION_B: &str = "session-bbbbbbbb";

fn activation(request_id: &str, session_id: &str, generation: u64) -> GatewayCommand {
    GatewayCommand::ProjectSession {
        request_id: request_id.into(),
        thread_id: "thread-a".into(),
        deadline_at: Utc::now() + TimeDelta::minutes(1),
        computer_session_id: session_id.into(),
        computer_session_generation: generation,
    }
}

fn computer(request_id: &str, session_id: &str, request: ComputerRequest) -> GatewayCommand {
    GatewayCommand::ProjectOperation {
        request_id: request_id.into(),
        thread_id: "thread-a".into(),
        deadline_at: Utc::now() + TimeDelta::minutes(1),
        computer_session_id: Some(session_id.into()),
        operation: ProjectOperation::Computer(request),
    }
}

#[tokio::test]
async fn cd1_stop_fails_closed_until_a_new_session_generation_rebinds() {
    let root = tempfile::tempdir().expect("project");
    let dispatcher = LocalProjectDispatcher::new();
    dispatcher.begin_connection(1).await.expect("connection");
    dispatcher
        .upsert("thread-a", root.path(), Utc::now() + TimeDelta::minutes(10))
        .await
        .expect("binding");
    assert!(matches!(
        dispatcher
            .execute(activation("activate-a", SESSION_A, 1), Some(1))
            .await,
        BridgeResult::ProjectSessionResult { .. }
    ));

    assert!(matches!(
        dispatcher
            .execute(
                computer("list-a", SESSION_A, ComputerRequest::ComputerListWindows),
                Some(1),
            )
            .await,
        BridgeResult::ProjectOperationResult {
            output: ProjectOperationOutput::Computer(ComputerOutput::ComputerWindows { .. }),
            ..
        }
    ));
    assert!(matches!(
        dispatcher
            .execute(
                computer("stop", SESSION_A, ComputerRequest::ComputerStop),
                Some(1),
            )
            .await,
        BridgeResult::ProjectOperationResult {
            output: ProjectOperationOutput::Computer(ComputerOutput::ComputerStop { .. }),
            ..
        }
    ));
    assert!(matches!(
        dispatcher
            .execute(
                computer("closed", SESSION_A, ComputerRequest::ComputerListWindows),
                Some(1),
            )
            .await,
        BridgeResult::OperationError { ref error_code, .. }
            if error_code == "computer_control_stopped"
    ));

    assert!(matches!(
        dispatcher
            .execute(activation("activate-b", SESSION_B, 2), Some(1))
            .await,
        BridgeResult::ProjectSessionResult { .. }
    ));
    assert!(matches!(
        dispatcher
            .execute(
                computer("list-b", SESSION_B, ComputerRequest::ComputerListWindows),
                Some(1),
            )
            .await,
        BridgeResult::ProjectOperationResult {
            output: ProjectOperationOutput::Computer(ComputerOutput::ComputerWindows { .. }),
            ..
        }
    ));
    dispatcher.retire_sessions().await.expect("retire");
}
