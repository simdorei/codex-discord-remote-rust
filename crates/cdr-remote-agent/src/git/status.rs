use cdr_core::deadline::RequestBudget;
use cdr_remote_protocol::output::CoreOutput;
use tokio::sync::watch;

use crate::files::ProjectFileAccess;

use super::GitError;
use super::process::run;

pub async fn repo_status(
    access: &ProjectFileAccess,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<CoreOutput, GitError> {
    let root = access.root();
    let branch = run(
        root,
        &strings(&["branch", "--show-current"]),
        cancelled.clone(),
        false,
        budget,
    )
    .await?
    .stdout
    .trim()
    .to_owned();
    let porcelain = run(
        root,
        &strings(&["status", "--porcelain=v1"]),
        cancelled.clone(),
        false,
        budget,
    )
    .await?
    .stdout;
    let (dirty, staged) = parse_status(&porcelain);
    let remotes = run(
        root,
        &strings(&["remote"]),
        cancelled.clone(),
        false,
        budget,
    )
    .await?
    .stdout
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_owned)
    .collect();
    let upstream_result = run(
        root,
        &strings(&[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ]),
        cancelled.clone(),
        true,
        budget,
    )
    .await?;
    let upstream = upstream_result
        .success
        .then(|| upstream_result.stdout.trim().to_owned());
    let (ahead, behind) = ahead_behind(root, upstream.as_deref(), cancelled, budget).await?;
    Ok(CoreOutput::RepoStatus {
        branch,
        dirty_files: visible(dirty),
        staged_files: visible(staged),
        remotes,
        upstream,
        ahead,
        behind,
    })
}

async fn ahead_behind(
    root: &std::path::Path,
    upstream: Option<&str>,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<(u64, u64), GitError> {
    if upstream.is_none() {
        return Ok((0, 0));
    }
    let output = run(
        root,
        &strings(&["rev-list", "--left-right", "--count", "HEAD...@{upstream}"]),
        cancelled,
        true,
        budget,
    )
    .await?;
    if !output.success {
        return Ok((0, 0));
    }
    let values = output.stdout.split_whitespace().collect::<Vec<_>>();
    if values.len() != 2 {
        return Ok((0, 0));
    }
    Ok((
        values[0].parse().unwrap_or(0),
        values[1].parse().unwrap_or(0),
    ))
}

fn parse_status(status: &str) -> (Vec<String>, Vec<String>) {
    let mut dirty = Vec::new();
    let mut staged = Vec::new();
    for line in status.lines().filter(|line| line.len() >= 4) {
        let bytes = line.as_bytes();
        let mut path = &line[3..];
        if let Some((_, destination)) = path.split_once(" -> ") {
            path = destination;
        }
        if bytes[0] == b'?' && bytes[1] == b'?' {
            dirty.push(path.to_owned());
        } else {
            if !matches!(bytes[0], b' ' | b'?') {
                staged.push(path.to_owned());
            }
            if !matches!(bytes[1], b' ' | b'?') {
                dirty.push(path.to_owned());
            }
        }
    }
    (dirty, staged)
}

fn visible(paths: Vec<String>) -> Vec<String> {
    paths
        .into_iter()
        .filter_map(|path| ProjectFileAccess::validate_path(&path).ok())
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}
