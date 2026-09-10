use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cdr_core::deadline::RequestBudget;
use cdr_remote_agent::terminal::{TerminalError, TerminalExecutionEngine};
use cdr_remote_protocol::identifiers::TerminalId;
use cdr_remote_protocol::output::TerminalOutput;
use cdr_remote_protocol::request::{TerminalRequest, TerminalShell};
use chrono::{TimeDelta, Utc};
use tokio::sync::watch;

fn request(terminal_id: Option<TerminalId>, command: impl Into<String>) -> TerminalRequest {
    TerminalRequest::TerminalExec {
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
    }
}

fn budget() -> RequestBudget {
    RequestBudget::from_deadline(Utc::now() + TimeDelta::seconds(60))
}

fn uncancelled() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

fn output(value: TerminalOutput) -> (TerminalId, Option<i64>, String, bool, bool) {
    match value {
        TerminalOutput::TerminalExec {
            terminal_id,
            exit_code,
            stdout,
            timed_out,
            cancelled,
            ..
        } => (terminal_id, exit_code, stdout, timed_out, cancelled),
        _ => panic!("expected terminal execution output"),
    }
}

#[tokio::test]
async fn t4_cancel_previous_replaces_only_the_same_terminal() {
    let project = tempfile::tempdir().expect("project");
    let engine = Arc::new(TerminalExecutionEngine::new(project.path(), "replace").expect("engine"));
    let (keep_alive, cancelled) = uncancelled();
    let seed = output(
        engine
            .execute(&request(None, "echo seed"), cancelled, &budget())
            .await
            .expect("seed"),
    );
    drop(keep_alive);
    let marker = project.path().join("started.txt");
    let long_engine = Arc::clone(&engine);
    let terminal_id = seed.0.clone();
    let worker = tokio::spawn(async move {
        let (keep_alive, cancelled) = uncancelled();
        let result = long_engine
            .execute(
                &request(Some(terminal_id), marker_command(&marker)),
                cancelled,
                &budget(),
            )
            .await;
        drop(keep_alive);
        result
    });
    wait_for(&project.path().join("started.txt")).await;
    let (keep_alive, cancelled) = uncancelled();
    let busy = engine
        .execute(
            &request(Some(seed.0.clone()), "echo busy"),
            cancelled,
            &budget(),
        )
        .await
        .expect_err("active terminal must reject overlap");
    drop(keep_alive);
    assert!(matches!(busy, TerminalError::ActiveCommand));

    let mut replacement = request(Some(seed.0), "echo replacement");
    let TerminalRequest::TerminalExec {
        cancel_previous, ..
    } = &mut replacement
    else {
        unreachable!()
    };
    *cancel_previous = true;
    let (keep_alive, cancelled) = uncancelled();
    let replacement = output(
        engine
            .execute(&replacement, cancelled, &budget())
            .await
            .expect("replacement"),
    );
    drop(keep_alive);
    let cancelled = output(worker.await.expect("worker").expect("long output"));
    assert!(cancelled.4);
    assert!(!cancelled.3);
    assert_eq!(replacement.1, Some(0));
    assert!(replacement.2.contains("replacement"));
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
