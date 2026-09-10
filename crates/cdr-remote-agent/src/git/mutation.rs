use cdr_core::deadline::RequestBudget;
use cdr_remote_protocol::output::CoreOutput;
use tokio::sync::watch;

use crate::files::ProjectFileAccess;
use crate::files::redaction::redact;

use super::GitError;
use super::process::{run, run_push};

const MAX_GIT_OUTPUT: usize = 200_000;

pub async fn commit(
    access: &ProjectFileAccess,
    message: &str,
    paths: &[String],
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<CoreOutput, GitError> {
    let paths = paths
        .iter()
        .map(|path| ProjectFileAccess::validate_path(path))
        .collect::<Result<Vec<_>, _>>()?;
    let untracked = run(
        access.root(),
        &strings(&["ls-files", "--others", "--exclude-standard"]),
        cancelled.clone(),
        false,
        budget,
    )
    .await?
    .stdout
    .lines()
    .map(str::to_owned)
    .collect::<std::collections::HashSet<_>>();
    let new_paths = paths
        .iter()
        .filter(|path| untracked.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    if !new_paths.is_empty() {
        let mut arguments = strings(&["add", "--intent-to-add", "--"]);
        arguments.extend(new_paths);
        run(access.root(), &arguments, cancelled.clone(), false, budget).await?;
    }
    let mut arguments = strings(&["commit", "--only", "-m", message, "--"]);
    arguments.extend(paths);
    run(access.root(), &arguments, cancelled.clone(), false, budget).await?;
    let commit = run(
        access.root(),
        &strings(&["rev-parse", "--short", "HEAD"]),
        cancelled.clone(),
        false,
        budget,
    )
    .await?
    .stdout
    .trim()
    .to_owned();
    let branch = current_branch(access, cancelled.clone(), budget).await?;
    let staged_files = run(
        access.root(),
        &strings(&["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]),
        cancelled,
        false,
        budget,
    )
    .await?
    .stdout
    .lines()
    .filter(|line| !line.is_empty())
    .map(str::to_owned)
    .collect();
    Ok(CoreOutput::GitCommit {
        commit,
        branch,
        staged_files,
    })
}

pub async fn push(
    access: &ProjectFileAccess,
    remote: &str,
    branch: Option<&str>,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<CoreOutput, GitError> {
    let remotes = run(
        access.root(),
        &strings(&["remote"]),
        cancelled.clone(),
        false,
        budget,
    )
    .await?
    .stdout;
    if !remotes.lines().any(|candidate| candidate.trim() == remote) {
        return Err(GitError::Failed(format!(
            "{remote}: Git remote is not configured"
        )));
    }
    let branch = match branch {
        Some(value) => value.to_owned(),
        None => current_branch(access, cancelled.clone(), budget).await?,
    };
    if branch.is_empty() {
        return Err(GitError::Failed(
            "detached HEAD cannot be pushed implicitly".into(),
        ));
    }
    let output = run_push(
        access.root(),
        &strings(&["push", "-u", "--", remote, &branch]),
        cancelled,
        budget,
    )
    .await?;
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    let combined = combined.trim();
    let end = combined.floor_char_boundary(MAX_GIT_OUTPUT.min(combined.len()));
    Ok(CoreOutput::GitPush {
        remote: remote.to_owned(),
        branch,
        output: redact(&combined[..end]),
    })
}

async fn current_branch(
    access: &ProjectFileAccess,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<String, GitError> {
    Ok(run(
        access.root(),
        &strings(&["branch", "--show-current"]),
        cancelled,
        false,
        budget,
    )
    .await?
    .stdout
    .trim()
    .to_owned())
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}
