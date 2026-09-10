use crate::ContextUsage;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
    time::{Duration, Instant},
};
use thiserror::Error;

#[derive(Clone, Copy, Debug)]
pub struct ContextReadBudget {
    pub max_bytes: u64,
    pub max_line_bytes: usize,
    pub max_duration: Duration,
}

impl Default for ContextReadBudget {
    fn default() -> Self {
        Self {
            max_bytes: 128 * 1024 * 1024,
            max_line_bytes: 8 * 1024 * 1024,
            max_duration: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Error)]
pub enum ContextReadError {
    #[error("context file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("context observation unavailable: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Usage(#[from] crate::ContextUsageError),
}

pub fn read_context_usage(
    path: &Path,
    thread: &str,
    budget: ContextReadBudget,
) -> Result<Option<ContextUsage>, ContextReadError> {
    read(path, thread, budget, |_| {})
}

#[derive(Debug)]
pub struct ContextSnapshot {
    pub usage: Option<ContextUsage>,
    pub recent_items: Vec<crate::context_text::ContextTextItem>,
}

pub fn read_context_snapshot(
    path: &Path,
    thread: &str,
    budget: ContextReadBudget,
    limit: usize,
) -> Result<ContextSnapshot, ContextReadError> {
    read_context_snapshot_with_mode(path, thread, budget, limit, crate::RecentTextMode::Visible)
}

pub fn read_context_snapshot_with_mode(
    path: &Path,
    thread: &str,
    budget: ContextReadBudget,
    limit: usize,
    mode: crate::RecentTextMode,
) -> Result<ContextSnapshot, ContextReadError> {
    let mut recent = crate::context_text::RecentText::new(limit, mode);
    let usage = read(path, thread, budget, |event| recent.push(event))?;
    Ok(ContextSnapshot {
        usage,
        recent_items: recent.finish(),
    })
}

fn read(
    path: &Path,
    thread: &str,
    budget: ContextReadBudget,
    mut observe: impl FnMut(&serde_json::Value),
) -> Result<Option<ContextUsage>, ContextReadError> {
    let start = Instant::now();
    let file = File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() > budget.max_bytes || budget.max_line_bytes == 0 {
        return Err(ContextReadError::Invalid(
            "file exceeds context read budget or is not a regular file",
        ));
    }
    let mut reader = BufReader::new(file.take(before.len()));
    let mut accumulator = crate::context_usage::Accumulator::default();
    let mut identified = false;
    let mut raw = Vec::new();
    loop {
        if start.elapsed() >= budget.max_duration {
            return Err(ContextReadError::Invalid(
                "context read time budget exceeded",
            ));
        }
        raw.clear();
        let cap = u64::try_from(budget.max_line_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let size = (&mut reader).take(cap).read_until(b'\n', &mut raw)?;
        if size == 0 {
            break;
        }
        if size > budget.max_line_bytes {
            return Err(ContextReadError::Invalid(
                "context record exceeds line budget",
            ));
        }
        if raw.last() != Some(&b'\n') {
            return Err(ContextReadError::Invalid(
                "context record is incomplete; writer may still be appending",
            ));
        }
        if raw.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let event: serde_json::Value = serde_json::from_slice(&raw)
            .map_err(|_| ContextReadError::Invalid("context record contains invalid JSON"))?;
        if !event.is_object() {
            return Err(ContextReadError::Invalid("context record is not an object"));
        }
        if !identified || event["type"] == "session_meta" {
            if event["type"] != "session_meta"
                || event
                    .pointer("/payload/id")
                    .and_then(serde_json::Value::as_str)
                    != Some(thread)
                || thread.is_empty()
            {
                return Err(ContextReadError::Invalid(
                    "session identity is absent or differs from the requested thread",
                ));
            }
            identified = true;
        }
        accumulator.push(&event)?;
        observe(&event);
    }
    if !identified {
        return Err(ContextReadError::Invalid(
            "session identity has not been recorded",
        ));
    }
    let after = reader.get_ref().get_ref().metadata()?;
    let current = path.metadata()?;
    if before.len() != after.len()
        || before.modified()? != after.modified()?
        || before.len() != current.len()
        || before.modified()? != current.modified()?
    {
        return Err(ContextReadError::Invalid(
            "session changed during context read; snapshot is unverified",
        ));
    }
    Ok(accumulator.finish())
}
