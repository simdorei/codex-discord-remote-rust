//! Deployment-only absence proof. Ordinary restart readiness has no exceptions.
use super::{RestartReadinessError, RestartReadinessState};
use cdr_app_server::AppServerClient;
use cdr_codex_state::CodexThreadStore;
use rusqlite::{Connection, OpenFlags};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub struct AbsentTarget {
    pub state_db: PathBuf,
    pub thread_id: String,
    pub room: i64,
    pub parent: i64,
}

impl AbsentTarget {
    pub(super) fn prove(&self, mirror_db: &Path) -> Result<Vec<String>, RestartReadinessError> {
        let fail = |reason: String| RestartReadinessError::InvalidThreadState {
            thread_id: self.thread_id.clone(),
            reason,
        };
        if self.thread_id.is_empty() || self.room <= 0 || self.parent <= 0 {
            return Err(fail("invalid maintenance target".into()));
        }
        if let Some(reason) = cdr_store::room_cleanup::pending_reason_pre_commentary_schema(
            mirror_db,
            self.room,
            Some(&self.thread_id),
        )? {
            return Err(fail(format!("maintenance target protected by {reason}")));
        }
        let store = CodexThreadStore::open(&self.state_db).map_err(|e| fail(e.to_string()))?;
        let mut ids = store
            .load_recent_threads(0)
            .map_err(|e| fail(e.to_string()))?
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        ids.sort();
        if ids.contains(&self.thread_id)
            || store
                .load_thread(&self.thread_id, true)
                .map_err(|e| fail(e.to_string()))?
                .is_some()
        {
            return Err(fail("maintenance target exists in Codex inventory".into()));
        }
        // No store initializer: this preflight must not migrate the live DB.
        let db = Connection::open_with_flags(mirror_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| fail(e.to_string()))?;
        db.busy_timeout(Duration::from_secs(2))
            .map_err(|e| fail(e.to_string()))?;
        for (table, query) in [
            (
                "codex_app_server_managed_targets",
                "SELECT EXISTS(SELECT 1 FROM codex_app_server_managed_targets WHERE thread_id=?1)",
            ),
            (
                "codex_thread_fork_handoffs",
                "SELECT EXISTS(SELECT 1 FROM codex_thread_fork_handoffs WHERE source_thread_id=?1 OR observed_target_thread_id=?1 OR target_thread_id=?1)",
            ),
        ] {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    [table],
                    |row| row.get(0),
                )
                .map_err(|e| fail(e.to_string()))?;
            if exists
                && db
                    .query_row(query, [&self.thread_id], |row| row.get::<_, bool>(0))
                    .map_err(|e| fail(e.to_string()))?
            {
                return Err(fail(format!(
                    "maintenance target retains ownership in {table}"
                )));
            }
        }
        let mut query = db.prepare("SELECT codex_thread_id,discord_thread_id,discord_channel_id FROM mirror_threads WHERE codex_thread_id=?1 OR discord_thread_id=?2")
            .map_err(|e| fail(e.to_string()))?;
        let rows = query
            .query_map((&self.thread_id, self.room), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| fail(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| fail(e.to_string()))?;
        if rows != vec![(self.thread_id.clone(), self.room, self.parent)] {
            return Err(fail(
                "maintenance mapping changed or room has multiple owners".into(),
            ));
        }
        Ok(ids)
    }
}

/// Only establishes preflight; never authorizes a stop without runtime-owned drain ACK.
pub async fn check_absent_maintenance(
    mirror_db: &Path,
    client: &AppServerClient,
    quiet: Duration,
    request_timeout: Duration,
    ticket: &AbsentTarget,
) -> Result<RestartReadinessState, RestartReadinessError> {
    super::check_readiness_inner(mirror_db, client, quiet, request_timeout, Some(ticket)).await
}
