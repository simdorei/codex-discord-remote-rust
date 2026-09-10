use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use serde_json::{Map, Value};

use crate::CodexStateError;

pub type SessionEvent = Map<String, Value>;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionTail {
    pub events: Vec<SessionEvent>,
    pub next_offset: u64,
}

pub fn read_new_session_events(
    path: &Path,
    start_offset: u64,
    max_events: Option<usize>,
) -> Result<SessionTail, CodexStateError> {
    if !path.is_file() {
        return Ok(SessionTail {
            events: Vec::new(),
            next_offset: start_offset,
        });
    }
    let mut reader = BufReader::new(File::open(path)?);
    reader.seek(SeekFrom::Start(start_offset))?;
    let mut events = Vec::new();
    let limit = max_events.unwrap_or_default();
    loop {
        let position = reader.stream_position()?;
        let mut raw = String::new();
        if reader.read_line(&mut raw)? == 0 {
            return Ok(SessionTail {
                events,
                next_offset: position,
            });
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = if let Ok(value) = serde_json::from_str(line) {
            value
        } else {
            reader.seek(SeekFrom::Start(position))?;
            return Ok(SessionTail {
                events,
                next_offset: position,
            });
        };
        if let Value::Object(event) = value {
            events.push(event);
        }
        if limit > 0 && events.len() >= limit {
            return Ok(SessionTail {
                events,
                next_offset: reader.stream_position()?,
            });
        }
    }
}

pub fn iter_session_events(path: &Path) -> Result<Vec<SessionEvent>, CodexStateError> {
    let mut events = Vec::new();
    for raw in BufReader::new(File::open(path)?).lines() {
        let raw = raw?;
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(raw.trim()) else {
            continue;
        };
        if let Value::Object(event) = value {
            events.push(event);
        }
    }
    Ok(events)
}
