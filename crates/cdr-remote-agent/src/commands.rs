mod catalog;
mod process;
mod sandbox;

use std::time::{Duration, Instant};

use cdr_remote_protocol::output::{CoreOutput, RiskTier};
use thiserror::Error;
use tokio::sync::watch;

use crate::files::redaction::redact;
use crate::files::{ProjectFileAccess, RemoteFileError};

pub use catalog::{DiscoveredCommand, discover_commands};
pub use process::{
    ProcessCompletion, ProcessError, ProcessOutcome, TRUNCATION_MARKER, run_bounded_process,
    run_bounded_process_cancellable,
};
pub use sandbox::{safe_environment, sandbox_arguments};

pub const MAX_OUTPUT_BYTES: usize = 12_000;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error(transparent)]
    File(#[from] RemoteFileError),
    #[error("{path}: {reason}")]
    Manifest { path: String, reason: String },
    #[error("{command_id}: {reason}")]
    Rejected { command_id: String, reason: String },
    #[error(transparent)]
    Process(#[from] ProcessError),
}

pub async fn run_command(
    access: &ProjectFileAccess,
    command_id: &str,
    timeout: Duration,
    codex_exe: Option<&str>,
) -> Result<CoreOutput, CommandError> {
    let (keep_alive, cancelled) = watch::channel(false);
    let result = run_command_cancellable(access, command_id, timeout, codex_exe, cancelled).await;
    drop(keep_alive);
    result
}

pub async fn run_command_cancellable(
    access: &ProjectFileAccess,
    command_id: &str,
    timeout: Duration,
    codex_exe: Option<&str>,
    cancelled: watch::Receiver<bool>,
) -> Result<CoreOutput, CommandError> {
    let commands = discover_commands(access)?;
    let selected = commands
        .iter()
        .find(|command| command.descriptor.command_id == command_id)
        .ok_or_else(|| {
            rejected(
                command_id,
                "command is not in a discovered project manifest",
            )
        })?;
    if matches!(
        selected.descriptor.risk_tier,
        RiskTier::Network | RiskTier::Destructive
    ) {
        return Err(rejected(
            command_id,
            format!(
                "{} command is not remotely executable",
                risk_name(selected.descriptor.risk_tier)
            ),
        ));
    }
    let arguments = sandbox_arguments(access.root(), &selected.arguments, codex_exe)?;
    let started = Instant::now();
    let outcome = run_bounded_process_cancellable(
        &arguments,
        access.root(),
        &safe_environment(),
        timeout,
        MAX_OUTPUT_BYTES,
        cancelled,
    )
    .await?;
    if outcome.completion == ProcessCompletion::Cancelled {
        return Err(rejected(
            command_id,
            "command was cancelled because the local bridge disconnected",
        ));
    }
    if outcome.completion == ProcessCompletion::TimedOut {
        return Err(rejected(
            command_id,
            format!("command timed out after {} seconds", timeout.as_secs_f64()),
        ));
    }
    Ok(CoreOutput::CommandRun {
        command_id: command_id.to_owned(),
        exit_code: i64::from(outcome.exit_code.unwrap_or(-1)),
        stdout: redact(&String::from_utf8_lossy(&outcome.stdout)),
        stderr: redact(&String::from_utf8_lossy(&outcome.stderr)),
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        truncated: outcome.stdout_truncated || outcome.stderr_truncated,
    })
}

fn rejected(command_id: &str, reason: impl Into<String>) -> CommandError {
    CommandError::Rejected {
        command_id: command_id.to_owned(),
        reason: reason.into(),
    }
}

fn risk_name(risk: RiskTier) -> &'static str {
    match risk {
        RiskTier::Read => "read",
        RiskTier::Verify => "verify",
        RiskTier::Network => "network",
        RiskTier::Destructive => "destructive",
    }
}
