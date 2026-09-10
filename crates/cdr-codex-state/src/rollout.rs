use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use regex::Regex;
use serde_json::Value;
use walkdir::WalkDir;

use crate::{CodexStateError, ThreadInfo};

#[derive(Debug, Default)]
struct RolloutState {
    source: String,
    thread_source: String,
    cwd: String,
    title: String,
    model: String,
    reasoning_effort: String,
}

pub fn parse_rollout_thread(
    path: &Path,
    thread_id: &str,
    session_names: Option<&BTreeMap<String, String>>,
) -> Result<Option<ThreadInfo>, CodexStateError> {
    let mut state = RolloutState {
        title: first_line(
            session_names
                .and_then(|names| names.get(thread_id))
                .map_or("", String::as_str),
        ),
        ..RolloutState::default()
    };
    for raw in BufReader::new(File::open(path)?).lines() {
        let raw = raw?;
        let Some((event_type, payload)) = rollout_payload(&raw) else {
            continue;
        };
        apply_payload(&mut state, &event_type, &payload);
        if !state.source.is_empty()
            && !state.cwd.is_empty()
            && !state.title.is_empty()
            && !state.model.is_empty()
        {
            break;
        }
    }
    if state.source != "vscode"
        || (!state.thread_source.is_empty() && state.thread_source != "user")
        || state.cwd.is_empty()
        || state.title.is_empty()
    {
        return Ok(None);
    }
    let updated_at = fs::metadata(path)?
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(Some(ThreadInfo {
        id: thread_id.to_owned(),
        title: state.title,
        cwd: state.cwd,
        updated_at: i64::try_from(updated_at).unwrap_or(i64::MAX),
        rollout_path: path.to_owned(),
        model: state.model,
        reasoning_effort: state.reasoning_effort,
        tokens_used: None,
        archived_at: 0,
    }))
}

pub fn load_missing_vscode_rollout_threads<S: std::hash::BuildHasher>(
    sessions_dir: &Path,
    existing_thread_ids: &std::collections::HashSet<String, S>,
    session_names: Option<&BTreeMap<String, String>>,
) -> Result<Vec<ThreadInfo>, CodexStateError> {
    if !sessions_dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = WalkDir::new(sessions_dir)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let mut paths: Vec<PathBuf> = entries
        .into_iter()
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("rollout-")
                        && Path::new(name)
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
                })
        })
        .collect();
    paths.sort();
    let mut threads = Vec::new();
    for path in paths {
        let Some(thread_id) = session_id_from_path(&path) else {
            continue;
        };
        if existing_thread_ids.contains(&thread_id) {
            continue;
        }
        if let Some(thread) = parse_rollout_thread(&path, &thread_id, session_names)? {
            threads.push(thread);
        }
    }
    Ok(threads)
}

fn rollout_payload(raw: &str) -> Option<(String, Value)> {
    let event: Value = serde_json::from_str(raw.trim()).ok()?;
    let payload = event.get("payload")?.as_object()?.clone();
    Some((
        event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        Value::Object(payload),
    ))
}

fn apply_payload(state: &mut RolloutState, event_type: &str, payload: &Value) {
    match event_type {
        "session_meta" => {
            update(&mut state.source, payload, "source");
            update(&mut state.thread_source, payload, "thread_source");
            update(&mut state.cwd, payload, "cwd");
        }
        "turn_context" => {
            update(&mut state.cwd, payload, "cwd");
            update(&mut state.model, payload, "model");
            update(&mut state.reasoning_effort, payload, "reasoning_effort");
            if let Some(settings) = payload
                .get("collaboration_mode")
                .and_then(|value| value.get("settings"))
            {
                update(&mut state.model, settings, "model");
                update(&mut state.reasoning_effort, settings, "reasoning_effort");
            }
        }
        _ if payload.get("type").and_then(Value::as_str) == Some("user_message")
            && state.title.is_empty() =>
        {
            state.title = first_line(
                payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
        }
        _ => {}
    }
}

fn update(target: &mut String, payload: &Value, key: &str) {
    if let Some(value) = payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        value.clone_into(target);
    }
}

fn first_line(text: &str) -> String {
    text.replace('\r', "\n")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

fn session_id_from_path(path: &Path) -> Option<String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            Regex::new(r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl$")
                .expect("valid regex")
        })
        .captures(path.file_name()?.to_str()?)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_owned())
}
