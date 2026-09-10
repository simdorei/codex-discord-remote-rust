//! One aggregate budget for multi-thread context inspection.
use crate::{ContextReadBudget, ContextReadError, ContextUsage};
use std::path::PathBuf;

pub struct ContextReadTarget {
    pub thread: String,
    pub path: PathBuf,
}

pub struct ContextEntry {
    pub thread: String,
    pub usage: Result<Option<ContextUsage>, ContextReadError>,
    pub recent_items: Vec<crate::ContextTextItem>,
}

pub struct ContextBatch {
    pub entries: Vec<ContextEntry>,
    pub skipped: usize,
}

#[must_use]
pub fn read_context_batch(
    targets: &[ContextReadTarget],
    budget: ContextReadBudget,
    max_files: usize,
) -> ContextBatch {
    read_context_batch_with_recent(targets, budget, max_files, None)
}

#[must_use]
pub fn read_context_batch_with_recent(
    targets: &[ContextReadTarget],
    budget: ContextReadBudget,
    max_files: usize,
    recent_limit: Option<usize>,
) -> ContextBatch {
    read_context_batch_with_mode(
        targets,
        budget,
        max_files,
        recent_limit,
        crate::RecentTextMode::Visible,
    )
}

#[must_use]
pub fn read_context_batch_with_mode(
    targets: &[ContextReadTarget],
    budget: ContextReadBudget,
    max_files: usize,
    recent_limit: Option<usize>,
    mode: crate::RecentTextMode,
) -> ContextBatch {
    let start = std::time::Instant::now();
    let count = targets.len().min(max_files.min(50));
    let mut remaining_bytes = budget.max_bytes;
    let entries = targets
        .iter()
        .take(count)
        .map(|target| {
            let mut recent_items = Vec::new();
            let usage = (|| {
                if start.elapsed() >= budget.max_duration {
                    return Err(ContextReadError::Invalid(
                        "aggregate context time budget exceeded",
                    ));
                }
                let length = target.path.metadata()?.len();
                if length > remaining_bytes {
                    return Err(ContextReadError::Invalid(
                        "aggregate context byte budget exceeded",
                    ));
                }
                // Charge attempted reads conservatively, including failures. The
                // per-file cap also rejects growth between this metadata and open.
                remaining_bytes -= length;
                let file_budget = ContextReadBudget {
                    max_bytes: length,
                    max_line_bytes: budget.max_line_bytes,
                    max_duration: budget.max_duration.saturating_sub(start.elapsed()),
                };
                if let Some(limit) = recent_limit {
                    let snapshot = crate::read_context_snapshot_with_mode(
                        &target.path,
                        &target.thread,
                        file_budget,
                        limit,
                        mode,
                    )?;
                    recent_items = snapshot.recent_items;
                    Ok(snapshot.usage)
                } else {
                    crate::read_context_usage(&target.path, &target.thread, file_budget)
                }
            })();
            ContextEntry {
                thread: target.thread.clone(),
                usage,
                recent_items,
            }
        })
        .collect();
    ContextBatch {
        entries,
        skipped: targets.len() - count,
    }
}
