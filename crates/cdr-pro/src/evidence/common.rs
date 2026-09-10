use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

pub fn receipt_key(session_id: &str, turn_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(session_id.as_bytes());
    digest.update([0]);
    digest.update(turn_id.as_bytes());
    hex::encode(digest.finalize())
}

pub fn probe_integrity_ok(plugin_root: &Path, relative: &Path, expected: &str) -> bool {
    let Ok(source) = std::fs::read(plugin_root.join(relative)) else {
        return false;
    };
    let canonical = normalize_newlines(&source);
    hex::encode(Sha256::digest(canonical)) == expected
}

fn normalize_newlines(source: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'\r' {
            result.push(b'\n');
            index += usize::from(source.get(index + 1) == Some(&b'\n')) + 1;
        } else {
            result.push(source[index]);
            index += 1;
        }
    }
    result
}

pub fn transcript_paths(codex_home: &Path, session_id: &str) -> Vec<PathBuf> {
    let suffix = format!("-{session_id}.jsonl");
    let mut paths = ["sessions", "archived_sessions"]
        .into_iter()
        .flat_map(|root| {
            WalkDir::new(codex_home.join(root))
                .follow_links(false)
                .into_iter()
                .filter_map(std::result::Result::ok)
        })
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy();
            (name.starts_with("rollout-") && name.ends_with(&suffix)).then(|| entry.into_path())
        })
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| {
        path.metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    paths
}

pub fn response_payload(line: &str) -> Option<Map<String, Value>> {
    let record: Value = serde_json::from_str(line).ok()?;
    let object = record.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }
    object.get("payload")?.as_object().cloned()
}

pub fn payload_turn_id(payload: &Map<String, Value>) -> Option<&str> {
    payload
        .get("internal_chat_message_metadata_passthrough")?
        .as_object()?
        .get("turn_id")?
        .as_str()
}

pub fn embedded_objects(raw: &Value) -> Vec<Map<String, Value>> {
    let mut strings = Vec::new();
    collect_strings(raw, &mut strings);
    strings.into_iter().flat_map(objects_in_text).collect()
}

fn collect_strings<'a>(raw: &'a Value, output: &mut Vec<&'a str>) {
    match raw {
        Value::String(text) => output.push(text),
        Value::Array(values) => {
            for value in values {
                collect_strings(value, output);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_strings(value, output);
            }
        }
        _ => {}
    }
}

fn objects_in_text(text: &str) -> Vec<Map<String, Value>> {
    text.match_indices('{')
        .filter_map(|(index, _)| {
            let mut deserializer = serde_json::Deserializer::from_str(&text[index..]);
            Value::deserialize(&mut deserializer)
                .ok()?
                .as_object()
                .cloned()
        })
        .collect()
}

pub fn read_json_object(path: &Path) -> Option<Map<String, Value>> {
    serde_json::from_str::<Value>(&std::fs::read_to_string(path).ok()?)
        .ok()?
        .as_object()
        .cloned()
}
