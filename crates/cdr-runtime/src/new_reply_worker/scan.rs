//! Bounded JSONL verification. A checkpoint belongs to one immutable file snapshot.
use cdr_codex_state::{CodexThreadStore, normalize_workspace_path};
use cdr_store::new_reply::NewReply;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata},
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};

const MAX_BYTES: u64 = 1_048_576;
const MAX_EVENTS: usize = 256;

#[derive(Default, Serialize, Deserialize)]
struct Cursor {
    #[serde(default)]
    generation: String,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    session_seen: bool,
    #[serde(default)]
    turn: Option<String>,
    #[serde(default)]
    matched: bool,
}

#[derive(Debug)]
pub(super) struct Scan {
    pub checkpoint: Value,
    pub verified: bool,
}

pub(super) fn inspect(record: &NewReply) -> Result<Scan, String> {
    if record.state == "verified" {
        return Ok(Scan {
            checkpoint: record.scan.clone(),
            verified: true,
        });
    }
    let id = &record.identity;
    let store = CodexThreadStore::open(Path::new(&id.state_db)).map_err(|e| e.to_string())?;
    let Some(thread) = store
        .load_thread(&id.thread_id, false)
        .map_err(|e| e.to_string())?
    else {
        return Ok(Scan {
            checkpoint: record.scan.clone(),
            verified: false,
        });
    };
    if normalize_workspace_path(&thread.cwd) != normalize_workspace_path(&id.cwd) {
        return Err("new thread persisted in a different project".into());
    }
    let path = Path::new(&thread.rollout_path);
    let result = inspect_file(path, record)?;
    let after = store
        .load_thread(&id.thread_id, false)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "new thread disappeared during verification".to_owned())?;
    if after.cwd != thread.cwd || after.rollout_path != thread.rollout_path {
        return Err("new thread source changed during verification".into());
    }
    Ok(result)
}

fn inspect_file(path: &Path, record: &NewReply) -> Result<Scan, String> {
    let mut file = File::open(path).map_err(|e| format!("new rollout open failed: {e}"))?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let generation = fingerprint(&before);
    let mut cursor: Cursor = serde_json::from_value(record.scan.clone())
        .map_err(|e| format!("invalid new verification checkpoint: {e}"))?;
    if cursor.generation != generation {
        cursor = Cursor {
            generation: generation.clone(),
            ..Cursor::default()
        };
    }
    file.seek(SeekFrom::Start(cursor.offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut consumed = 0;
    let mut verified = cursor.matched;
    for _ in 0..MAX_EVENTS {
        if verified {
            break;
        }
        let Some(end) = bytes[consumed..].iter().position(|b| *b == b'\n') else {
            break;
        };
        let next = consumed + end + 1;
        if next > usize::try_from(MAX_BYTES).unwrap_or(usize::MAX) {
            break;
        }
        let line = &bytes[consumed..next - 1];
        let event: Value = serde_json::from_slice(line)
            .map_err(|e| format!("invalid new rollout JSON at byte {}: {e}", cursor.offset))?;
        consumed = next;
        verified = observe(&event, &mut cursor, record)?;
        if verified || started.elapsed() >= Duration::from_millis(20) {
            break;
        }
    }
    if consumed == 0 && u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err("new rollout line exceeds the 1 MiB verification budget".into());
    }
    cursor.offset += u64::try_from(consumed).map_err(|e| e.to_string())?;
    cursor.matched = verified;
    if fingerprint(&file.metadata().map_err(|e| e.to_string())?) != generation
        || fingerprint(&fs::metadata(path).map_err(|e| e.to_string())?) != generation
    {
        // An append, rewrite, replacement or truncation invalidates this snapshot.
        return Ok(Scan {
            checkpoint: serde_json::to_value(Cursor::default()).map_err(|e| e.to_string())?,
            verified: false,
        });
    }
    Ok(Scan {
        checkpoint: serde_json::to_value(cursor).map_err(|e| e.to_string())?,
        verified,
    })
}

fn observe(event: &Value, cursor: &mut Cursor, record: &NewReply) -> Result<bool, String> {
    let Some(payload) = event.get("payload") else {
        return Ok(false);
    };
    match event.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            if payload.get("id").and_then(Value::as_str) != Some(&record.identity.thread_id)
                || payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(normalize_workspace_path)
                    != Some(normalize_workspace_path(&record.identity.cwd))
            {
                return Err("new rollout thread/project identity mismatch".into());
            }
            cursor.session_seen = true;
        }
        Some("turn_context") => {
            cursor.turn = payload
                .get("turn_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        Some("event_msg")
            if payload.get("type").and_then(Value::as_str) == Some("task_started") =>
        {
            cursor.turn = payload
                .get("turn_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        Some("event_msg")
            if payload.get("type").and_then(Value::as_str) == Some("task_complete") =>
        {
            cursor.turn = None;
        }
        _ => {}
    }
    if !cursor.session_seen || cursor.turn.is_none() || cursor.turn != record.turn_id {
        return Ok(false);
    }
    let text = match event.get("type").and_then(Value::as_str) {
        Some("event_msg")
            if payload.get("type").and_then(Value::as_str) == Some("user_message") =>
        {
            payload
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }
        Some("response_item") if payload.get("role").and_then(Value::as_str) == Some("user") => {
            payload
                .get("content")
                .and_then(Value::as_array)
                .map(|content| {
                    content
                        .iter()
                        .filter_map(|item| item.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("")
                })
        }
        _ => None,
    };
    Ok(text.is_some_and(|text| {
        hex::encode(Sha256::digest(text.as_bytes())) == record.identity.prompt_sha256
    }))
}

fn fingerprint(metadata: &Metadata) -> String {
    format!(
        "{}:{:?}:{:?}:{}",
        metadata.len(),
        metadata.created(),
        metadata.modified(),
        file_identity(metadata)
    )
}
#[cfg(windows)]
fn file_identity(metadata: &Metadata) -> String {
    use std::os::windows::fs::MetadataExt;
    format!(
        "{}:{}",
        metadata.creation_time(),
        metadata.file_attributes()
    )
}
#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!("{}:{}", metadata.dev(), metadata.ino())
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
