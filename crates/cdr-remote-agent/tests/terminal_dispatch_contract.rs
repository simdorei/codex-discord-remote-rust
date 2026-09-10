use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_protocol::identifiers::TerminalId;
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::output::{ProjectOperationOutput, TerminalOutput};
use cdr_remote_protocol::request::{ProjectOperation, TerminalRequest, TerminalShell};
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

fn terminal_command(
    request_id: &str,
    session_id: &str,
    terminal_id: Option<TerminalId>,
    command: impl Into<String>,
) -> GatewayCommand {
    GatewayCommand::ProjectOperation {
        request_id: request_id.into(),
        thread_id: "thread-a".into(),
        deadline_at: Utc::now() + TimeDelta::minutes(2),
        computer_session_id: Some(session_id.into()),
        operation: ProjectOperation::Terminal(TerminalRequest::TerminalExec {
            terminal_id,
            shell: if cfg!(windows) {
                TerminalShell::Powershell
            } else {
                TerminalShell::Sh
            },
            command: command.into(),
            cwd: None,
            environment: BTreeMap::default(),
            timeout_seconds: 60,
            cancel_previous: false,
        }),
    }
}

fn output(result: BridgeResult) -> (TerminalId, Option<i64>, String, bool) {
    let BridgeResult::ProjectOperationResult {
        output:
            ProjectOperationOutput::Terminal(TerminalOutput::TerminalExec {
                terminal_id,
                exit_code,
                stdout,
                cancelled,
                ..
            }),
        ..
    } = result
    else {
        panic!("expected terminal execution result")
    };
    (terminal_id, exit_code, stdout, cancelled)
}

async fn dispatcher(root: &Path) -> Arc<LocalProjectDispatcher> {
    let dispatcher = Arc::new(LocalProjectDispatcher::new());
    dispatcher.begin_connection(1).await.expect("connection");
    dispatcher
        .upsert("thread-a", root, Utc::now() + TimeDelta::minutes(10))
        .await
        .expect("binding");
    assert!(matches!(
        dispatcher
            .execute(activation("activate-a", SESSION_A, 1), Some(1))
            .await,
        BridgeResult::ProjectSessionResult { .. }
    ));
    dispatcher
}

#[tokio::test]
async fn td1_dispatch_executes_and_reuses_a_session_owned_terminal() {
    let project = tempfile::tempdir().expect("project");
    let dispatcher = dispatcher(project.path()).await;
    let first = output(
        dispatcher
            .execute(
                terminal_command("first", SESSION_A, None, "echo dispatch-ok"),
                Some(1),
            )
            .await,
    );
    let second = output(
        dispatcher
            .execute(
                terminal_command("second", SESSION_A, Some(first.0.clone()), "echo reused"),
                Some(1),
            )
            .await,
    );
    assert_eq!(first.1, Some(0));
    assert!(first.2.contains("dispatch-ok"));
    assert_eq!(second.0, first.0);
    assert!(second.2.contains("reused"));
}

#[tokio::test]
async fn td2_session_replacement_cancels_owned_terminal_process_tree() {
    let project = tempfile::tempdir().expect("project");
    let dispatcher = dispatcher(project.path()).await;
    let seed = output(
        dispatcher
            .execute(
                terminal_command("seed", SESSION_A, None, "echo seed"),
                Some(1),
            )
            .await,
    );
    let marker = project.path().join("session-replace-started.txt");
    let long_command = terminal_command("long", SESSION_A, Some(seed.0), marker_command(&marker));
    let worker_dispatcher = Arc::clone(&dispatcher);
    let worker =
        tokio::spawn(async move { worker_dispatcher.execute(long_command, Some(1)).await });
    wait_for(&marker).await;

    let activated = dispatcher
        .execute(activation("activate-b", SESSION_B, 2), Some(1))
        .await;
    assert!(matches!(
        activated,
        BridgeResult::ProjectSessionResult { .. }
    ));
    let stopped = output(worker.await.expect("worker"));
    assert!(stopped.3);

    let stale = dispatcher
        .execute(
            terminal_command("stale", SESSION_A, None, "echo stale"),
            Some(1),
        )
        .await;
    assert!(matches!(
        stale,
        BridgeResult::OperationError { ref error_code, .. } if error_code == "computer_control"
    ));
    let fresh = output(
        dispatcher
            .execute(
                terminal_command("fresh", SESSION_B, None, "echo fresh"),
                Some(1),
            )
            .await,
    );
    assert_eq!(fresh.1, Some(0));
}

fn marker_command(path: &Path) -> String {
    if cfg!(windows) {
        format!(
            "Set-Content -LiteralPath '{}' -Value x; Start-Sleep -Seconds 30",
            path.display().to_string().replace('\'', "''")
        )
    } else {
        format!("touch '{}'; sleep 30", path.display())
    }
}

async fn wait_for(path: &Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("marker was not created");
}
