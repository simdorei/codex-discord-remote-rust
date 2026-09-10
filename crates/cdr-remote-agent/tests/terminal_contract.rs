use cdr_core::deadline::RequestBudget;
use cdr_remote_agent::terminal::{TerminalError, TerminalExecutionEngine};
use cdr_remote_protocol::identifiers::TerminalId;
use cdr_remote_protocol::output::{TerminalCwdScope, TerminalOutput};
use cdr_remote_protocol::request::{TerminalRequest, TerminalShell};
use chrono::{TimeDelta, Utc};
use tokio::sync::watch;

fn request(terminal_id: Option<TerminalId>, command: impl Into<String>) -> TerminalRequest {
    TerminalRequest::TerminalExec {
        terminal_id,
        shell: shell(),
        command: command.into(),
        cwd: None,
        environment: BTreeMap::default(),
        timeout_seconds: 30,
        cancel_previous: false,
    }
}

fn shell() -> TerminalShell {
    if cfg!(windows) {
        TerminalShell::Powershell
    } else {
        TerminalShell::Sh
    }
}

fn output(value: TerminalOutput) -> ExecOutput {
    match value {
        TerminalOutput::TerminalExec {
            terminal_id,
            exit_code,
            stdout,
            cwd,
            timed_out,
            cancelled,
            receipt,
            ..
        } => ExecOutput {
            terminal_id,
            exit_code,
            stdout,
            cwd,
            timed_out,
            cancelled,
            cwd_scope: receipt.cwd_scope,
            command_digest: receipt.command_digest,
        },
        _ => panic!("expected terminal execution output"),
    }
}

struct ExecOutput {
    terminal_id: TerminalId,
    exit_code: Option<i64>,
    stdout: String,
    cwd: String,
    timed_out: bool,
    cancelled: bool,
    cwd_scope: TerminalCwdScope,
    command_digest: String,
}

fn budget(seconds: i64) -> RequestBudget {
    RequestBudget::from_deadline(Utc::now() + TimeDelta::seconds(seconds))
}

fn uncancelled() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

fn cwd_and_env_command() -> &'static str {
    if cfg!(windows) {
        "Write-Output (Get-Location).Path; Write-Output $env:TERMINAL_QA; Write-Output 'api_key=Abcd1234Efgh5678Ijkl'"
    } else {
        "pwd; printf '%s\\n' \"$TERMINAL_QA\"; printf '%s\\n' 'api_key=Abcd1234Efgh5678Ijkl'"
    }
}

fn env_command() -> &'static str {
    if cfg!(windows) {
        "Write-Output $env:TERMINAL_QA"
    } else {
        "printf '%s\\n' \"$TERMINAL_QA\""
    }
}

#[tokio::test]
async fn t1_terminal_reuses_external_cwd_and_environment_with_safe_receipt() {
    let project = tempfile::tempdir().expect("project");
    let external = tempfile::tempdir().expect("external");
    let engine = TerminalExecutionEngine::new(project.path(), "session-a").expect("engine");
    let mut first_request = request(None, cwd_and_env_command());
    let TerminalRequest::TerminalExec {
        cwd, environment, ..
    } = &mut first_request
    else {
        unreachable!()
    };
    *cwd = Some(external.path().display().to_string());
    environment.insert("TERMINAL_QA".into(), "persisted".into());
    let (keep_alive, cancelled) = uncancelled();
    let first = output(
        engine
            .execute(&first_request, cancelled, &budget(30))
            .await
            .expect("first execution"),
    );
    drop(keep_alive);

    let (keep_alive, cancelled) = uncancelled();
    let second = output(
        engine
            .execute(
                &request(Some(first.terminal_id.clone()), env_command()),
                cancelled,
                &budget(30),
            )
            .await
            .expect("second execution"),
    );
    drop(keep_alive);

    assert_eq!(first.exit_code, Some(0));
    assert_eq!(first.cwd_scope, TerminalCwdScope::ExternalAbsolute);
    assert_eq!(first.cwd, external.path().display().to_string());
    assert!(first.stdout.contains("persisted"));
    assert!(first.stdout.contains("api_key=[REDACTED]"));
    assert_eq!(second.cwd, first.cwd);
    assert!(second.stdout.contains("persisted"));
    assert!(!first.stdout.contains(&first.command_digest));
}

#[tokio::test]
async fn t2_terminal_ids_are_owned_by_one_session_engine() {
    let project = tempfile::tempdir().expect("project");
    let owner = TerminalExecutionEngine::new(project.path(), "owner").expect("owner");
    let stranger = TerminalExecutionEngine::new(project.path(), "stranger").expect("stranger");
    let (keep_alive, cancelled) = uncancelled();
    let created = output(
        owner
            .execute(&request(None, "echo owned"), cancelled, &budget(30))
            .await
            .expect("owner execution"),
    );
    drop(keep_alive);
    let (keep_alive, cancelled) = uncancelled();
    let error = stranger
        .execute(
            &request(Some(created.terminal_id), "echo escaped"),
            cancelled,
            &budget(30),
        )
        .await
        .expect_err("foreign terminal must fail");
    drop(keep_alive);
    assert!(matches!(error, TerminalError::ForeignTerminal));
}

#[tokio::test]
async fn t3_timeout_returns_a_receipted_outcome() {
    let project = tempfile::tempdir().expect("project");
    let engine = TerminalExecutionEngine::new(project.path(), "timeout").expect("engine");
    let command = if cfg!(windows) {
        "Write-Output started; Start-Sleep -Seconds 30"
    } else {
        "printf 'started\\n'; sleep 30"
    };
    let mut terminal_request = request(None, command);
    let TerminalRequest::TerminalExec {
        timeout_seconds, ..
    } = &mut terminal_request
    else {
        unreachable!()
    };
    *timeout_seconds = 1;
    let (keep_alive, cancelled) = uncancelled();
    let result = output(
        engine
            .execute(&terminal_request, cancelled, &budget(5))
            .await
            .expect("timeout output"),
    );
    drop(keep_alive);
    assert_eq!(result.exit_code, None);
    assert!(result.timed_out);
    assert!(!result.cancelled);
    assert!(result.stdout.contains("started"));
}
use std::collections::BTreeMap;
