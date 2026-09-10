use super::SessionEvent;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

pub(super) fn turn_from_event(event: &SessionEvent) -> Option<&str> {
    let payload = event.get("payload")?;
    if event.get("type").and_then(Value::as_str) == Some("turn_context")
        || payload.get("type").and_then(Value::as_str) == Some("task_started")
    {
        payload
            .get("turn_id")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
    } else {
        None
    }
}

/// Upgrade a legacy cursor once, without replaying any old messages.
pub(crate) fn read_context_before(path: &Path, cursor: u64) -> std::io::Result<Option<String>> {
    let mut current = None;
    for line in BufReader::new(File::open(path)?.take(cursor)).lines() {
        let line = line?;
        let prefix: String = line.chars().take(200).collect();
        if !(prefix.contains("turn_context") || prefix.contains("task_started")) {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<SessionEvent>(&line)
            && let Some(turn) = turn_from_event(&event)
        {
            current = Some(turn.to_owned());
        }
    }
    Ok(current)
}
