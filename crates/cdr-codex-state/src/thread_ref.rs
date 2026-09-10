use std::collections::BTreeMap;

use thiserror::Error;

use crate::{ThreadInfo, normalize_workspace_path, strip_windows_extended_prefix};

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ThreadResolveError {
    #[error("No {kind}Codex threads found in the local state DB.", kind = if *.archived { "archived " } else { "" })]
    NoThreads { archived: bool },
    #[error("No alternate thread found.")]
    NoAlternate,
    #[error("{kind}thread index out of range: {reference}", kind = if *.archived { "Archived " } else { "Thread " })]
    IndexOutOfRange { reference: String, archived: bool },
    #[error("Multiple threads match reference `{reference}`. Use one exact ID: {options}")]
    Ambiguous { reference: String, options: String },
    #[error("Thread not found: {0}")]
    NotFound(String),
}

pub fn resolve_thread_ref<'a>(
    threads: &'a [ThreadInfo],
    thread_ref: &str,
    selected_thread_id: Option<&str>,
    archived: bool,
) -> Result<&'a ThreadInfo, ThreadResolveError> {
    if threads.is_empty() {
        return Err(ThreadResolveError::NoThreads { archived });
    }
    let reference = thread_ref.trim();
    let normalized = reference.to_lowercase();
    if let Some(thread) = threads.iter().find(|thread| thread.id == reference) {
        return Ok(thread);
    }
    if is_full_uuid(reference) {
        return threads
            .iter()
            .find(|t| t.id.eq_ignore_ascii_case(reference))
            .ok_or_else(|| ThreadResolveError::NotFound(reference.into()));
    }
    if matches!(normalized.as_str(), "other" | "next") {
        return threads
            .iter()
            .find(|thread| Some(thread.id.as_str()) != selected_thread_id)
            .ok_or(ThreadResolveError::NoAlternate);
    }
    if reference
        .chars()
        .all(|character| character.is_ascii_digit())
    {
        let index = reference.parse::<usize>().unwrap_or_default();
        return threads
            .get(index.saturating_sub(1))
            .filter(|_| index > 0)
            .ok_or_else(|| ThreadResolveError::IndexOutOfRange {
                reference: reference.into(),
                archived,
            });
    }
    let refs = workspace_ref_map(threads);
    let normalized_path = normalize_workspace_path(reference);
    // A spelling can belong to several namespaces. Do not let the first
    // matching namespace silently select another conversation.
    let matches = threads
        .iter()
        .filter(|thread| {
            (is_short_id(reference) && thread.id.to_lowercase().starts_with(&normalized))
                || refs
                    .get(&thread.id)
                    .is_some_and(|alias| alias.to_lowercase() == normalized)
                || normalize_workspace_path(&thread.cwd) == normalized_path
                || workspace_name(thread).to_lowercase() == normalized
        })
        .collect::<Vec<_>>();
    unique_reference(&matches, reference)?
        .ok_or_else(|| ThreadResolveError::NotFound(reference.into()))
}

fn unique_reference<'a>(
    matches: &[&'a ThreadInfo],
    reference: &str,
) -> Result<Option<&'a ThreadInfo>, ThreadResolveError> {
    match matches {
        [] => Ok(None),
        [thread] => Ok(Some(*thread)),
        _ => Err(ThreadResolveError::Ambiguous {
            reference: reference.into(),
            options: matches
                .iter()
                .take(10)
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

fn is_full_uuid(reference: &str) -> bool {
    reference.len() == 36
        && reference.bytes().enumerate().all(|(index, value)| {
            if [8, 13, 18, 23].contains(&index) {
                value == b'-'
            } else {
                value.is_ascii_hexdigit()
            }
        })
}

fn is_short_id(reference: &str) -> bool {
    reference.len() >= 4 && reference.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

#[must_use]
pub fn workspace_ref_map(threads: &[ThreadInfo]) -> BTreeMap<String, String> {
    let mut totals = BTreeMap::<String, usize>::new();
    for thread in threads {
        *totals
            .entry(workspace_name(thread).to_lowercase())
            .or_default() += 1;
    }
    let mut seen = BTreeMap::<String, usize>::new();
    threads
        .iter()
        .map(|thread| {
            let workspace = workspace_name(thread);
            let key = workspace.to_lowercase();
            let number = seen
                .entry(key.clone())
                .and_modify(|value| *value += 1)
                .or_insert(1);
            let reserved = matches!(key.as_str(), "other" | "next")
                || key.chars().all(|c| c.is_ascii_digit())
                || is_full_uuid(&key)
                || is_short_id(&key);
            let reference = if reserved || totals.get(&key).copied().unwrap_or_default() > 1 {
                format!("{workspace}:{number}")
            } else {
                workspace
            };
            (thread.id.clone(), reference)
        })
        .collect::<BTreeMap<_, _>>()
        .iter()
        .map(|(id, alias)| {
            let collision = !alias
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
                || threads.iter().any(|other| {
                    other.id != *id
                        && (other.id.to_lowercase() == alias.to_lowercase()
                            || workspace_name(other).to_lowercase() == alias.to_lowercase())
                });
            (
                id.clone(),
                if collision { id.clone() } else { alias.clone() },
            )
        })
        .collect()
}

fn workspace_name(thread: &ThreadInfo) -> String {
    let cwd = strip_windows_extended_prefix(thread.cwd.trim());
    cwd.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(if cwd.is_empty() { "-" } else { &cwd })
        .to_owned()
}
