use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use super::{ArchiveDeleteError, io_error};

pub(super) fn validate_json_state(path: &Path) -> Result<(), ArchiveDeleteError> {
    if !path.exists() {
        return Ok(());
    }
    let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let data: Value = serde_json::from_slice(bytes).map_err(|source| ArchiveDeleteError::Json {
        path: path.into(),
        source,
    })?;
    if !data.is_object() {
        return Err(json_object_error(path));
    }
    Ok(())
}

pub(super) fn scrub_json_state(
    path: &Path,
    thread_id: &str,
    bridge: bool,
    backup_dir: &Path,
) -> Result<Vec<String>, ArchiveDeleteError> {
    let Some(original) = read_backed_up_bytes(path, backup_dir)? else {
        return Ok(Vec::new());
    };
    let bytes = original
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(&original);
    let mut data = serde_json::from_slice::<Value>(bytes)
        .map_err(|source| ArchiveDeleteError::Json {
            path: path.into(),
            source,
        })?
        .as_object()
        .cloned()
        .ok_or_else(|| json_object_error(path))?;
    let changed = if bridge {
        scrub_bridge(&mut data, thread_id)
    } else {
        scrub_global(&mut data, thread_id)
    };
    if !changed.is_empty() {
        save_json(path, &data, &original)?;
    }
    Ok(changed)
}

pub(super) fn scrub_session_index(
    path: &Path,
    thread_id: &str,
    backup_dir: &Path,
) -> Result<usize, ArchiveDeleteError> {
    let Some(bytes) = read_backed_up_bytes(path, backup_dir)? else {
        return Ok(0);
    };
    let original = String::from_utf8(bytes).map_err(|e| {
        io_error(
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        )
    })?;
    let mut kept = Vec::new();
    let mut removed = 0;
    for line in original.lines() {
        let matches = serde_json::from_str::<Value>(line.trim())
            .ok()
            .is_some_and(|value| value.get("id").and_then(Value::as_str) == Some(thread_id));
        if matches {
            removed += 1;
        } else {
            kept.push(line);
        }
    }
    if removed > 0 {
        let mut rewritten = kept.join("\n");
        if !rewritten.is_empty() && original.ends_with(['\n', '\r']) {
            rewritten.push('\n');
        }
        super::metadata_write::replace_checked(path, rewritten.as_bytes(), original.as_bytes())?;
    }
    Ok(removed)
}

fn read_backed_up_bytes(
    path: &Path,
    backup_dir: &Path,
) -> Result<Option<Vec<u8>>, ArchiveDeleteError> {
    let backup = backup_dir.join(
        path.file_name()
            .ok_or(ArchiveDeleteError::ConcurrentMutation)?,
    );
    let source_exists = path.try_exists().map_err(|e| io_error(path, e))?;
    let backup_exists = backup.try_exists().map_err(|e| io_error(&backup, e))?;
    if !source_exists && !backup_exists {
        return Ok(None);
    }
    if source_exists != backup_exists {
        return Err(ArchiveDeleteError::ConcurrentMutation);
    }
    // The expected bytes come from the recovery copy, never a fresh unbacked-up
    // source snapshot. replace_checked compares these bytes again under its lock.
    let original = fs::read(&backup).map_err(|e| io_error(&backup, e))?;
    if fs::read(path).map_err(|e| io_error(path, e))? != original {
        return Err(ArchiveDeleteError::ConcurrentMutation);
    }
    Ok(Some(original))
}

fn scrub_bridge(data: &mut Map<String, Value>, thread_id: &str) -> Vec<String> {
    let mut changed = Vec::new();
    if data.get("selected_thread_id").and_then(Value::as_str) == Some(thread_id) {
        data.remove("selected_thread_id");
        changed.push("selected_thread_id".into());
    }
    if remove_object_key(data, "recent_live_approval_requests", thread_id) {
        changed.push("recent_live_approval_requests".into());
    }
    let matching_recent = data
        .get("recent_ui_thread")
        .and_then(Value::as_object)
        .and_then(|value| value.get("thread_id"))
        .and_then(Value::as_str)
        == Some(thread_id);
    if matching_recent {
        data.remove("recent_ui_thread");
        changed.push("recent_ui_thread".into());
    }
    changed
}

fn scrub_global(data: &mut Map<String, Value>, thread_id: &str) -> Vec<String> {
    let mut changed = Vec::new();
    if remove_object_key(data, "queued-follow-ups", thread_id) {
        changed.push("queued-follow-ups".into());
    }
    if let Some(items) = data
        .get_mut("pinned-thread-ids")
        .and_then(Value::as_array_mut)
    {
        let before = items.len();
        items.retain(|value| value.as_str() != Some(thread_id));
        if items.len() != before {
            changed.push("pinned-thread-ids".into());
        }
    }
    changed
}

fn remove_object_key(data: &mut Map<String, Value>, field: &str, key: &str) -> bool {
    let Some(values) = data.get_mut(field).and_then(Value::as_object_mut) else {
        return false;
    };
    let removed = values.remove(key).is_some();
    if removed && values.is_empty() {
        data.remove(field);
    }
    removed
}

fn save_json(
    path: &Path,
    data: &Map<String, Value>,
    original: &[u8],
) -> Result<(), ArchiveDeleteError> {
    let mut bytes = serde_json::to_vec_pretty(data).map_err(|source| ArchiveDeleteError::Json {
        path: path.into(),
        source,
    })?;
    bytes.push(b'\n');
    super::metadata_write::replace_checked(path, &bytes, original)
}

fn json_object_error(path: &Path) -> ArchiveDeleteError {
    ArchiveDeleteError::Json {
        path: path.into(),
        source: serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected a JSON object",
        )),
    }
}
