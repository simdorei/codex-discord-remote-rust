use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::Result;
use crate::schema::open_initialized;

pub const HISTORY_POLL_TARGET_LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryTargetSource {
    Startup,
    Allowed,
    MirrorProject,
    MirrorThread,
}

impl HistoryTargetSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Allowed => "allowed",
            Self::MirrorProject => "mirror_project",
            Self::MirrorThread => "mirror_thread",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryPollTarget {
    pub source: HistoryTargetSource,
    pub channel_id: u64,
}

pub fn history_poll_targets(
    path: &Path,
    allowed_channel_ids: &BTreeSet<u64>,
    startup_channel_id: Option<u64>,
) -> Result<Vec<HistoryPollTarget>> {
    let connection = open_initialized(path)?;
    let mut targets = Vec::with_capacity(HISTORY_POLL_TARGET_LIMIT);
    let mut seen = HashSet::with_capacity(HISTORY_POLL_TARGET_LIMIT);

    add_target(
        &mut targets,
        &mut seen,
        HistoryTargetSource::Startup,
        startup_channel_id,
    );
    for channel_id in allowed_channel_ids {
        add_target(
            &mut targets,
            &mut seen,
            HistoryTargetSource::Allowed,
            Some(*channel_id),
        );
    }

    if targets.len() < HISTORY_POLL_TARGET_LIMIT {
        let mut statement = connection.prepare(
            "SELECT discord_channel_id FROM mirror_projects \
             ORDER BY updated_at DESC, project_key ASC",
        )?;
        let rows = statement.query_map([], read_discord_id)?;
        for row in rows {
            add_target(
                &mut targets,
                &mut seen,
                HistoryTargetSource::MirrorProject,
                row?,
            );
            if targets.len() == HISTORY_POLL_TARGET_LIMIT {
                break;
            }
        }
    }

    if targets.len() < HISTORY_POLL_TARGET_LIMIT {
        let mut statement = connection.prepare(
            "SELECT discord_thread_id FROM mirror_threads \
             ORDER BY updated_at DESC, codex_thread_id ASC",
        )?;
        let rows = statement.query_map([], read_discord_id)?;
        for row in rows {
            add_target(
                &mut targets,
                &mut seen,
                HistoryTargetSource::MirrorThread,
                row?,
            );
            if targets.len() == HISTORY_POLL_TARGET_LIMIT {
                break;
            }
        }
    }

    Ok(targets)
}

fn add_target(
    targets: &mut Vec<HistoryPollTarget>,
    seen: &mut HashSet<u64>,
    source: HistoryTargetSource,
    channel_id: Option<u64>,
) {
    let Some(channel_id) = channel_id.filter(|value| *value != 0) else {
        return;
    };
    if targets.len() >= HISTORY_POLL_TARGET_LIMIT || !seen.insert(channel_id) {
        return;
    }
    targets.push(HistoryPollTarget { source, channel_id });
}

fn read_discord_id(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(0)?.map_or(Ok(None), |value| {
        u64::try_from(value).map(Some).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })
    })
}
