use cdr_remote_protocol::output::{CoreOutput, RuleFile, SearchMatch};
use thiserror::Error;
use tokio::sync::watch;

use crate::files::redaction::redact;
use crate::files::{MAX_FILE_BYTES, ProjectFileAccess, RemoteFileError};

const RULE_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md", ".codex/config.toml"];
const MAX_RULE_CHARACTERS: usize = 20_000;

#[derive(Debug, Error)]
pub enum CodeError {
    #[error(transparent)]
    File(#[from] RemoteFileError),
    #[error("project inspection was cancelled because the local bridge disconnected")]
    Cancelled,
}

pub fn project_rules(
    access: &ProjectFileAccess,
    cancelled: &watch::Receiver<bool>,
) -> Result<CoreOutput, CodeError> {
    let mut rules = Vec::new();
    for candidate in RULE_FILES {
        ensure_active(cancelled)?;
        if !access.file_exists(candidate)? {
            continue;
        }
        let raw = match access.read_bytes(candidate, MAX_FILE_BYTES) {
            Ok(raw) => raw,
            Err(RemoteFileError::Size { .. } | RemoteFileError::UnsafePath { .. }) => continue,
            Err(error) => return Err(error.into()),
        };
        let Ok(content) = String::from_utf8(raw) else {
            continue;
        };
        let end = content.floor_char_boundary(MAX_RULE_CHARACTERS.min(content.len()));
        rules.push(RuleFile {
            path: (*candidate).to_owned(),
            content: redact(&content[..end]),
        });
    }
    Ok(CoreOutput::ProjectRules { rules })
}

pub fn search(
    access: &ProjectFileAccess,
    query: &str,
    max_results: u16,
    cancelled: &watch::Receiver<bool>,
) -> Result<CoreOutput, CodeError> {
    let mut matches = Vec::new();
    for relative in access.scan_paths()? {
        ensure_active(cancelled)?;
        if matches.len() >= usize::from(max_results) {
            break;
        }
        let path = relative.to_string_lossy().replace('\\', "/");
        let Ok(raw) = access.read_bytes(&path, MAX_FILE_BYTES) else {
            continue;
        };
        if raw.contains(&0) {
            continue;
        }
        let Ok(content) = String::from_utf8(raw) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            if !line.contains(query) {
                continue;
            }
            let line = line.trim();
            let end = line.floor_char_boundary(400.min(line.len()));
            matches.push(SearchMatch {
                path: path.clone(),
                line: index as u64 + 1,
                snippet: redact(&line[..end]),
            });
            if matches.len() >= usize::from(max_results) {
                break;
            }
        }
    }
    Ok(CoreOutput::CodeSearch { matches })
}

fn ensure_active(cancelled: &watch::Receiver<bool>) -> Result<(), CodeError> {
    if *cancelled.borrow() {
        Err(CodeError::Cancelled)
    } else {
        Ok(())
    }
}
