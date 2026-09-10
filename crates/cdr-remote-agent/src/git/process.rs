use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use cdr_core::deadline::RequestBudget;
use regex::Regex;
use tokio::sync::watch;

use crate::commands::{ProcessCompletion, run_bounded_process_cancellable, safe_environment};
use crate::files::redaction::redact;

use super::GitError;

const MAX_GIT_CAPTURE_BYTES: usize = 400_000;

pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub truncated: bool,
}

pub async fn run(
    root: &Path,
    arguments: &[String],
    cancelled: watch::Receiver<bool>,
    allow_failure: bool,
    budget: &RequestBudget,
) -> Result<GitOutput, GitError> {
    run_policy(
        root,
        arguments,
        cancelled,
        allow_failure,
        false,
        Duration::from_mins(2),
        &[],
        budget,
    )
    .await
}

pub async fn run_patch(
    root: &Path,
    arguments: &[String],
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<GitOutput, GitError> {
    run_policy(
        root,
        arguments,
        cancelled,
        false,
        true,
        Duration::from_mins(2),
        &[],
        budget,
    )
    .await
}

pub async fn run_push(
    root: &Path,
    arguments: &[String],
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<GitOutput, GitError> {
    let helpers = trusted_helpers(root, cancelled.clone(), budget).await?;
    run_policy(
        root,
        arguments,
        cancelled,
        false,
        false,
        Duration::from_mins(5),
        &helpers,
        budget,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_policy(
    root: &Path,
    arguments: &[String],
    cancelled: watch::Receiver<bool>,
    allow_failure: bool,
    allow_truncated: bool,
    timeout: Duration,
    helpers: &[String],
    budget: &RequestBudget,
) -> Result<GitOutput, GitError> {
    let command = policy_command(root, arguments, helpers);
    let mut environment = safe_environment();
    environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    environment.insert("GIT_CONFIG_GLOBAL".into(), null_device().into());
    let output = execute(&command, root, &environment, cancelled, timeout, budget).await?;
    if output.truncated && !allow_truncated {
        return Err(GitError::Failed(
            "Git command output exceeded the safe capture limit".into(),
        ));
    }
    if !output.success && !allow_failure {
        return Err(failed(&output));
    }
    Ok(output)
}

async fn trusted_helpers(
    root: &Path,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<Vec<String>, GitError> {
    let mut helpers = Vec::new();
    for scope in ["--system", "--global"] {
        let command = strings(&["git", "config", scope, "--get-all", "credential.helper"]);
        let mut environment = safe_environment();
        environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        let output = execute(
            &command,
            root,
            &environment,
            cancelled.clone(),
            Duration::from_secs(10),
            budget,
        )
        .await?;
        if output.truncated || (!output.success && !output.stderr.trim().is_empty()) {
            continue;
        }
        helpers.extend(
            output
                .stdout
                .lines()
                .map(str::trim)
                .filter(|value| helper_pattern().is_match(value))
                .map(str::to_owned),
        );
    }
    helpers.sort();
    helpers.dedup();
    Ok(helpers)
}

async fn execute(
    command: &[String],
    root: &Path,
    environment: &std::collections::HashMap<String, String>,
    cancelled: watch::Receiver<bool>,
    cap: Duration,
    budget: &RequestBudget,
) -> Result<GitOutput, GitError> {
    let timeout = budget
        .remaining(Some(cap))
        .map_err(|_| GitError::TimedOut)?;
    let outcome = run_bounded_process_cancellable(
        command,
        root,
        environment,
        timeout,
        MAX_GIT_CAPTURE_BYTES,
        cancelled,
    )
    .await?;
    match outcome.completion {
        ProcessCompletion::TimedOut => return Err(GitError::TimedOut),
        ProcessCompletion::Cancelled => return Err(GitError::Cancelled),
        ProcessCompletion::Exited => {}
    }
    Ok(GitOutput {
        stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
        success: outcome.exit_code == Some(0),
        truncated: outcome.stdout_truncated || outcome.stderr_truncated,
    })
}

fn policy_command(root: &Path, arguments: &[String], helpers: &[String]) -> Vec<String> {
    let mut command = strings(&[
        "git",
        "-c",
        &format!("core.hooksPath={}", null_device()),
        "-c",
        "core.fsmonitor=false",
        "-c",
        "commit.gpgSign=false",
        "-c",
        "credential.helper=",
        "-c",
        &format!("safe.directory={}", root.display()),
    ]);
    for helper in helpers {
        command.extend(["-c".into(), format!("credential.helper={helper}")]);
    }
    command.extend_from_slice(arguments);
    command
}

fn failed(output: &GitOutput) -> GitError {
    let message = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    let end = message.floor_char_boundary(4_000.min(message.len()));
    GitError::Failed(redact(&message[..end]))
}

fn helper_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"^[A-Za-z0-9._/+:-]+(?:\s+--?[A-Za-z0-9._/=:+-]+)*$")
            .expect("valid helper pattern")
    })
}

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}
