use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row};

use crate::{CodexStateError, ThreadInfo};

pub struct CodexThreadStore {
    path: PathBuf,
}

impl CodexThreadStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, CodexStateError> {
        let path = path.into();
        if !path.is_file() {
            return Err(CodexStateError::StateDatabaseMissing(path));
        }
        let _ = open_read_only(&path)?;
        Ok(Self { path })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_recent_threads(&self, limit: u32) -> Result<Vec<ThreadInfo>, CodexStateError> {
        self.query_threads(
            "WHERE archived = 0 ORDER BY updated_at DESC, id",
            limit,
            false,
        )
    }

    /// Exact identity lookup is independent of any displayed recent-list limit.
    pub fn load_thread(
        &self,
        id: &str,
        archived: bool,
    ) -> Result<Option<ThreadInfo>, CodexStateError> {
        let connection = open_read_only(&self.path)?;
        Ok(connection.query_row("SELECT id,title,cwd,updated_at,rollout_path,model,reasoning_effort,tokens_used,archived_at FROM threads WHERE id=?1 AND archived=?2",(id,i64::from(archived)),|row|thread_from_row(row,archived)).optional()?)
    }

    pub fn load_user_root_threads(&self, limit: u32) -> Result<Vec<ThreadInfo>, CodexStateError> {
        self.query_threads(
            "WHERE archived = 0 AND source = 'vscode' AND COALESCE(thread_source, '') IN ('', 'user') AND title != '' ORDER BY updated_at DESC",
            limit,
            false,
        )
    }

    pub fn load_archived_threads(&self, limit: u32) -> Result<Vec<ThreadInfo>, CodexStateError> {
        self.query_threads(
            "WHERE archived = 1 ORDER BY archived_at DESC, updated_at DESC, id",
            limit,
            true,
        )
    }

    fn query_threads(
        &self,
        clause: &str,
        limit: u32,
        archived: bool,
    ) -> Result<Vec<ThreadInfo>, CodexStateError> {
        let archived_column = if archived { ", archived_at" } else { "" };
        let mut query = format!(
            "SELECT id, title, cwd, updated_at, rollout_path, model, reasoning_effort, tokens_used{archived_column} FROM threads {clause}"
        );
        if limit > 0 {
            query.push_str(" LIMIT ?1");
        }
        let connection = open_read_only(&self.path)?;
        let mut statement = connection.prepare(&query)?;
        let threads = if limit > 0 {
            statement
                .query_map([limit], |row| thread_from_row(row, archived))?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            statement
                .query_map([], |row| thread_from_row(row, archived))?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(threads)
    }
}

fn open_read_only(path: &Path) -> Result<Connection, rusqlite::Error> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

fn thread_from_row(row: &Row<'_>, archived: bool) -> rusqlite::Result<ThreadInfo> {
    Ok(ThreadInfo {
        id: row.get(0)?,
        title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        cwd: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        updated_at: row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
        rollout_path: PathBuf::from(row.get::<_, Option<String>>(4)?.unwrap_or_default()),
        model: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        reasoning_effort: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        tokens_used: row.get(7)?,
        archived_at: if archived {
            row.get::<_, Option<i64>>(8)?.unwrap_or_default()
        } else {
            0
        },
    })
}
