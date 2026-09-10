use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::CodexStateError;

pub fn load_active_workspace_roots(path: &Path) -> Result<Vec<PathBuf>, CodexStateError> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let data: Value = serde_json::from_slice(&fs::read(path)?)?;
    Ok(data
        .get("active-workspace-roots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect())
}

pub fn load_session_thread_names(path: &Path) -> Result<BTreeMap<String, String>, CodexStateError> {
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(path)?;
    let mut mapping = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = value
            .get("thread_name")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            continue;
        };
        if !name.is_empty() {
            mapping.insert(id.to_owned(), name.to_owned());
        }
    }
    Ok(mapping)
}

#[must_use]
pub fn strip_windows_extended_prefix(path: &str) -> String {
    let value = path.trim();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        value.to_owned()
    }
}

#[must_use]
pub fn normalize_workspace_path(path: &str) -> String {
    let stripped = strip_windows_extended_prefix(path);
    if stripped.is_empty() {
        return String::new();
    }
    let mut normalized = PathBuf::new();
    for component in Path::new(&stripped).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let value = normalized.to_string_lossy().into_owned();
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

#[must_use]
pub fn normalize_ui_match_text(text: &str) -> String {
    text.replace('\r', "\n")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

#[must_use]
pub fn build_ui_name_prefixes(text: &str) -> Vec<String> {
    let text = normalize_ui_match_text(text);
    if text.is_empty() {
        return Vec::new();
    }
    let mut candidates = vec![text.clone()];
    for limit in [120, 96, 72, 56, 40] {
        if text.chars().count() > limit {
            let value: String = text.chars().take(limit).collect();
            let value = value.trim_end_matches([' ', '.', ',', ';', ':', '!', '?', '-']);
            if !value.is_empty() {
                candidates.push(value.to_owned());
            }
        }
    }
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|value| seen.insert(value.to_lowercase()))
        .collect()
}
