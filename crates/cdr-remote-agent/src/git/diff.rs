use std::fmt::Write as _;

use cdr_core::deadline::RequestBudget;
use cdr_remote_protocol::output::{CoreOutput, DiffFile};
use similar::TextDiff;
use tokio::sync::watch;

use crate::files::redaction::redact;
use crate::files::{MAX_FILE_BYTES, ProjectFileAccess, RemoteFileError};

use super::GitError;
use super::process::{run, run_patch};

const MAX_GIT_OUTPUT: usize = 200_000;
const MAX_UNTRACKED_FILES: usize = 10_000;

pub async fn repo_diff(
    access: &ProjectFileAccess,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<CoreOutput, GitError> {
    let tracked_paths = visible_tracked(access, cancelled.clone(), budget).await?;
    let mut files = Vec::new();
    let mut patch = String::new();
    let mut truncated = false;
    if !tracked_paths.is_empty() {
        let mut arguments = strings(&["diff", "--numstat", "HEAD", "--"]);
        arguments.extend(tracked_paths.clone());
        let numeric = run(access.root(), &arguments, cancelled.clone(), false, budget).await?;
        files.extend(parse_numstat(&numeric.stdout));
        let mut arguments = strings(&["diff", "--no-ext-diff", "HEAD", "--"]);
        arguments.extend(tracked_paths);
        let tracked = run_patch(access.root(), &arguments, cancelled.clone(), budget).await?;
        truncated |= tracked.truncated;
        append_bounded(&mut patch, &tracked.stdout, &mut truncated);
    }
    append_untracked(
        access,
        cancelled,
        budget,
        &mut files,
        &mut patch,
        &mut truncated,
    )
    .await?;
    let total_added = files.iter().map(|file| file.added).sum::<u64>();
    let total_removed = files.iter().map(|file| file.removed).sum::<u64>();
    let summary = format!(
        "{} file(s) changed, +{total_added}/-{total_removed}",
        files.len()
    );
    Ok(CoreOutput::RepoDiff {
        files,
        summary,
        patch: redact(&patch),
        truncated,
    })
}

async fn visible_tracked(
    access: &ProjectFileAccess,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<Vec<String>, GitError> {
    let output = run(
        access.root(),
        &strings(&["diff", "--name-only", "-z", "HEAD", "--"]),
        cancelled,
        false,
        budget,
    )
    .await?;
    Ok(output
        .stdout
        .split('\0')
        .filter(|path| !path.is_empty())
        .filter_map(|path| ProjectFileAccess::validate_path(path).ok())
        .collect())
}

async fn append_untracked(
    access: &ProjectFileAccess,
    cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
    files: &mut Vec<DiffFile>,
    patch: &mut String,
    truncated: &mut bool,
) -> Result<(), GitError> {
    let output = run(
        access.root(),
        &strings(&["ls-files", "--others", "--exclude-standard", "-z"]),
        cancelled,
        false,
        budget,
    )
    .await?;
    for (index, raw_path) in output
        .stdout
        .split('\0')
        .filter(|path| !path.is_empty())
        .enumerate()
    {
        if index >= MAX_UNTRACKED_FILES {
            return Err(RemoteFileError::Limit {
                pattern: "<git>".into(),
                reason: format!("untracked file count exceeds {MAX_UNTRACKED_FILES}"),
            }
            .into());
        }
        let Ok(relative_path) = ProjectFileAccess::validate_path(raw_path) else {
            continue;
        };
        let Ok(raw) = access.read_bytes(&relative_path, MAX_FILE_BYTES) else {
            continue;
        };
        let Ok(content) = std::str::from_utf8(&raw) else {
            files.push(DiffFile {
                path: relative_path.clone(),
                added: 0,
                removed: 0,
            });
            let mut message = String::new();
            let _ = writeln!(message, "Binary untracked file: {relative_path}");
            append_bounded(patch, &message, truncated);
            continue;
        };
        if raw.contains(&0) {
            files.push(DiffFile {
                path: relative_path.clone(),
                added: 0,
                removed: 0,
            });
            let mut message = String::new();
            let _ = writeln!(message, "Binary untracked file: {relative_path}");
            append_bounded(patch, &message, truncated);
            continue;
        }
        files.push(DiffFile {
            path: relative_path.clone(),
            added: content.lines().count() as u64,
            removed: 0,
        });
        let fragment = TextDiff::from_lines("", content)
            .unified_diff()
            .header("/dev/null", &format!("b/{relative_path}"))
            .to_string();
        append_bounded(patch, &fragment, truncated);
    }
    Ok(())
}

fn parse_numstat(raw: &str) -> Vec<DiffFile> {
    raw.lines()
        .filter_map(|line| {
            let parts = line.splitn(3, '\t').collect::<Vec<_>>();
            (parts.len() == 3).then(|| DiffFile {
                path: parts[2].to_owned(),
                added: parts[0].parse().unwrap_or(0),
                removed: parts[1].parse().unwrap_or(0),
            })
        })
        .collect()
}

fn append_bounded(output: &mut String, value: &str, truncated: &mut bool) {
    let remaining = MAX_GIT_OUTPUT.saturating_sub(output.len());
    let end = value.floor_char_boundary(remaining.min(value.len()));
    output.push_str(&value[..end]);
    *truncated |= end < value.len();
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}
