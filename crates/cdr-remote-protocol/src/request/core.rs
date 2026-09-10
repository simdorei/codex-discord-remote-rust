use std::sync::OnceLock;

use cdr_core::{Validate, ValidationError, ValidationResult, count, length};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::file_change::FileApplyPatchRequest;

fn default_search_results() -> u16 {
    100
}
fn default_command_timeout() -> u16 {
    60
}
fn default_remote() -> String {
    "origin".to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreRequest {
    ProjectRules,
    ProjectStatus,
    CodeSearch {
        query: String,
        #[serde(default = "default_search_results")]
        max_results: u16,
    },
    FileApplyPatch {
        changes: Vec<crate::file_change::FileChange>,
    },
    FileCreate {
        path: String,
        content: String,
        #[serde(default)]
        overwrite: bool,
    },
    CommandList,
    CommandRun {
        command_id: String,
        #[serde(default = "default_command_timeout")]
        timeout_seconds: u16,
    },
    RepoStatus,
    RepoDiff,
    GitCommit {
        message: String,
        paths: Vec<String>,
    },
    GitPush {
        #[serde(default = "default_remote")]
        remote: String,
        #[serde(default)]
        branch: Option<String>,
    },
    SaveImage {
        path: String,
        data_base64: String,
        #[serde(default)]
        overwrite: bool,
    },
    SaveImageFromUrl {
        path: String,
        url: String,
        #[serde(default)]
        overwrite: bool,
    },
    ListImages,
    RetrieveImage {
        path: String,
    },
    CheckpointList,
    CheckpointShow {
        checkpoint_id: String,
    },
    CheckpointRestore {
        checkpoint_id: String,
    },
}

impl Validate for CoreRequest {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::ProjectRules
            | Self::ProjectStatus
            | Self::CommandList
            | Self::RepoStatus
            | Self::RepoDiff
            | Self::ListImages
            | Self::CheckpointList => Ok(()),
            Self::CodeSearch { query, max_results } => {
                length("query", query, 1, 500)?;
                range("max_results", *max_results, 1, 200)
            }
            Self::FileApplyPatch { changes } => FileApplyPatchRequest {
                changes: changes.clone(),
            }
            .validate(),
            Self::FileCreate { path, content, .. } => {
                length("path", path, 1, 1_000)?;
                length("content", content, 0, 1_048_576)
            }
            Self::CommandRun {
                command_id,
                timeout_seconds,
            } => {
                length("command_id", command_id, 1, 300)?;
                range("timeout_seconds", *timeout_seconds, 1, 300)
            }
            Self::GitCommit { message, paths } => {
                length("message", message, 1, 500)?;
                count("paths", paths.len(), 1, 200)?;
                for path in paths {
                    length("paths", path, 0, usize::MAX)?;
                }
                Ok(())
            }
            Self::GitPush { remote, branch } => validate_git_target(remote, branch.as_deref()),
            Self::SaveImage {
                path, data_base64, ..
            } => {
                length("path", path, 1, 1_000)?;
                length("data_base64", data_base64, 4, 7_000_000)
            }
            Self::SaveImageFromUrl { path, url, .. } => {
                length("path", path, 1, 1_000)?;
                let parsed = url::Url::parse(url)
                    .map_err(|error| ValidationError::new("url", error.to_string()))?;
                if matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some() {
                    Ok(())
                } else {
                    Err(ValidationError::new("url", "must be an HTTP or HTTPS URL"))
                }
            }
            Self::RetrieveImage { path } => length("path", path, 1, 1_000),
            Self::CheckpointShow { checkpoint_id } | Self::CheckpointRestore { checkpoint_id } => {
                match_id("checkpoint_id", checkpoint_id, r"^cp_[a-f0-9]{16}$")
            }
        }
    }
}

fn range(field: &'static str, value: u16, min: u16, max: u16) -> ValidationResult {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(ValidationError::new(
            field,
            format!("must be between {min} and {max}"),
        ))
    }
}

fn validate_git_target(remote: &str, branch: Option<&str>) -> ValidationResult {
    match_id("remote", remote, r"^[A-Za-z0-9._-]+$")?;
    if let Some(branch) = branch {
        length("branch", branch, 0, 250)?;
        match_id("branch", branch, r"^[A-Za-z0-9][A-Za-z0-9._/-]*$")?;
    }
    Ok(())
}

fn match_id(field: &'static str, value: &str, expression: &'static str) -> ValidationResult {
    static PATTERNS: OnceLock<std::sync::Mutex<std::collections::HashMap<&'static str, Regex>>> =
        OnceLock::new();
    let patterns = PATTERNS.get_or_init(Default::default);
    let mut guard = patterns.lock().expect("regex cache lock");
    let pattern = guard
        .entry(expression)
        .or_insert_with(|| Regex::new(expression).expect("valid regex"));
    if pattern.is_match(value) {
        Ok(())
    } else {
        Err(ValidationError::new(field, "invalid value"))
    }
}
