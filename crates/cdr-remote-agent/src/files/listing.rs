use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use cdr_remote_protocol::message::{FileEntry, ListFilesOutput};

use super::path::{is_sensitive, relative_text};
use super::platform::{RootGuard, is_reparse};
use super::{MAX_LIST_RESULTS, RemoteFileError};

const MAX_LIST_SCAN_CANDIDATES: usize = 10_000;
const SKIP_DIRECTORIES: &[&str] = &[
    ".codex-remote-mcp",
    ".git",
    ".next",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "vendor",
];

pub fn list_files(
    guard: &RootGuard,
    pattern: &str,
    limit: usize,
) -> Result<ListFilesOutput, RemoteFileError> {
    validate_pattern(pattern)?;
    let candidates = scan_paths(guard)?
        .into_iter()
        .filter(|relative| glob_matches(pattern, &relative_text(relative)))
        .collect::<Vec<_>>();
    let bounded_limit = limit.clamp(1, MAX_LIST_RESULTS);
    let entries = candidates
        .into_iter()
        .filter_map(|relative| {
            let locked = guard.open_regular(&relative).ok()?;
            let size = locked.file.metadata().ok()?.len();
            Some(FileEntry {
                path: relative_text(&relative),
                size_bytes: size,
            })
        })
        .collect::<Vec<_>>();
    let truncated = entries.len() > bounded_limit;
    let files = entries.into_iter().take(bounded_limit).collect();
    Ok(ListFilesOutput { files, truncated })
}

pub fn scan_paths(guard: &RootGuard) -> Result<Vec<PathBuf>, RemoteFileError> {
    guard.verify()?;
    let mut scan_count = 0;
    let mut candidates = Vec::new();
    visit(
        guard,
        Path::new(""),
        &mut scan_count,
        &mut candidates,
        "**/*",
    )?;
    candidates.sort_by(|left, right| {
        let left = relative_text(left);
        let right = relative_text(right);
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then(left.cmp(&right))
    });
    candidates.dedup();
    Ok(candidates)
}

fn visit(
    guard: &RootGuard,
    directory: &Path,
    scan_count: &mut usize,
    output: &mut Vec<PathBuf>,
    pattern: &str,
) -> Result<(), RemoteFileError> {
    let (entries, _locks) = guard.read_directory(directory)?;
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left = left.file_name().to_string_lossy().into_owned();
        let right = right.file_name().to_string_lossy().into_owned();
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then(left.cmp(&right))
    });
    let mut names = HashSet::new();
    for entry in entries {
        *scan_count += 1;
        if *scan_count > MAX_LIST_SCAN_CANDIDATES {
            return Err(RemoteFileError::Limit {
                pattern: pattern.to_owned(),
                reason: format!(
                    "glob scanned too many candidates; limit is {MAX_LIST_SCAN_CANDIDATES}"
                ),
            });
        }
        let name = entry.file_name();
        if !names.insert(name.clone()) {
            continue;
        }
        let relative = directory.join(name);
        if is_sensitive(&relative) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if is_reparse(&metadata) || metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let basename = relative
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if !SKIP_DIRECTORIES.contains(&basename.as_str()) {
                visit(guard, &relative, scan_count, output, pattern)?;
            }
        } else if metadata.is_file() && glob_matches(pattern, &relative_text(&relative)) {
            output.push(relative);
        }
    }
    Ok(())
}

fn validate_pattern(pattern: &str) -> Result<(), RemoteFileError> {
    let parts = pattern.split('/').collect::<Vec<_>>();
    let invalid = pattern.is_empty()
        || pattern.contains('\0')
        || pattern.contains('\\')
        || pattern.starts_with('/')
        || pattern.as_bytes().get(1) == Some(&b':')
        || parts.contains(&"..")
        || parts.iter().filter(|part| **part == "**").count() > 1;
    if invalid {
        Err(RemoteFileError::UnsafePattern {
            pattern: pattern.to_owned(),
            reason: "relative glob patterns without '..' segments are required".into(),
        })
    } else {
        Ok(())
    }
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let patterns = pattern.split('/').collect::<Vec<_>>();
    let values = value.split('/').collect::<Vec<_>>();
    match_segments(&patterns, &values)
}

fn match_segments(patterns: &[&str], values: &[&str]) -> bool {
    match patterns {
        [] => values.is_empty(),
        ["**", tail @ ..] => {
            match_segments(tail, values)
                || (!values.is_empty() && match_segments(patterns, &values[1..]))
        }
        [head, tail @ ..] => {
            !values.is_empty()
                && wildcard_component(head, values[0])
                && match_segments(tail, &values[1..])
        }
    }
}

fn wildcard_component(pattern: &str, value: &str) -> bool {
    let pattern = normalized(pattern);
    let value = normalized(value);
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut states = vec![false; value.len() + 1];
    states[0] = true;
    for token in pattern {
        let mut next = vec![false; value.len() + 1];
        if *token == b'*' {
            next[0] = states[0];
            for index in 1..=value.len() {
                next[index] = next[index - 1] || states[index];
            }
        } else {
            for index in 1..=value.len() {
                next[index] = states[index - 1] && (*token == b'?' || *token == value[index - 1]);
            }
        }
        states = next;
    }
    states[value.len()]
}

fn normalized(value: &str) -> String {
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value.to_owned()
    }
}
