use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use cdr_core::deadline::RequestBudget;
use cdr_remote_protocol::identifiers::{TerminalId, TerminalReceiptId};
use cdr_remote_protocol::output::{TerminalExecutionReceipt, TerminalOutput};
use cdr_remote_protocol::request::TerminalShell;
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::{ExecutionTicket, TerminalEntry, TerminalExecutionEngine, generated_id};
use crate::commands::{ProcessCompletion, run_bounded_process_cancellable};
use crate::files::redaction::redact;
use crate::terminal::shell::{arguments, cwd_scope, display_path, resolve_cwd};
use crate::terminal::{MAX_TERMINAL_STREAM_BYTES, TerminalError};

impl TerminalExecutionEngine {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_reserved(
        &self,
        terminal_id: &TerminalId,
        entry: &TerminalEntry,
        ticket: &ExecutionTicket,
        requested_shell: TerminalShell,
        command: &str,
        requested_cwd: Option<&str>,
        requested_environment: &BTreeMap<String, String>,
        timeout_seconds: u16,
        cancelled: watch::Receiver<bool>,
        budget: &RequestBudget,
    ) -> Result<TerminalOutput, TerminalError> {
        let (current_cwd, mut environment) = {
            let state = entry.state.lock().await;
            (state.cwd.clone(), state.environment.clone())
        };
        let cwd = resolve_cwd(&current_cwd, requested_cwd)?;
        environment.extend(
            requested_environment
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        let (shell, argv) = arguments(requested_shell, command)?;
        let timeout = budget.remaining(Some(Duration::from_secs(u64::from(timeout_seconds))))?;
        let relay = tokio::spawn(relay_cancellation(cancelled, ticket.cancel.clone()));
        let started = Instant::now();
        let outcome = run_bounded_process_cancellable(
            &argv,
            &cwd,
            &environment,
            timeout,
            MAX_TERMINAL_STREAM_BYTES,
            ticket.cancel.subscribe(),
        )
        .await;
        relay.abort();
        let outcome = outcome?;
        {
            let mut state = entry.state.lock().await;
            state.cwd.clone_from(&cwd);
            state.environment = environment;
        }
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let timed_out = outcome.completion == ProcessCompletion::TimedOut;
        let cancelled = outcome.completion == ProcessCompletion::Cancelled;
        let truncated = outcome.stdout_truncated || outcome.stderr_truncated;
        let exit_code = outcome.exit_code.map(i64::from);
        let receipt = TerminalExecutionReceipt {
            receipt_id: TerminalReceiptId(generated_id("tr_")),
            terminal_id: terminal_id.clone(),
            command_digest: format!("{:x}", Sha256::digest(command.as_bytes())),
            shell,
            cwd_scope: cwd_scope(&self.root, &cwd),
            exit_code,
            stdout_bytes: outcome.stdout_bytes,
            stderr_bytes: outcome.stderr_bytes,
            duration_ms,
            timed_out,
            cancelled,
            truncated,
        };
        Ok(TerminalOutput::TerminalExec {
            terminal_id: terminal_id.clone(),
            process_id: u64::from(outcome.process_id),
            exit_code,
            stdout: redact(&String::from_utf8_lossy(&outcome.stdout)),
            stderr: redact(&String::from_utf8_lossy(&outcome.stderr)),
            cwd: display_path(&cwd),
            duration_ms,
            timed_out,
            cancelled,
            truncated,
            receipt,
        })
    }
}

async fn relay_cancellation(mut source: watch::Receiver<bool>, target: watch::Sender<bool>) {
    loop {
        if *source.borrow() {
            target.send_replace(true);
            return;
        }
        if source.changed().await.is_err() {
            return;
        }
    }
}
