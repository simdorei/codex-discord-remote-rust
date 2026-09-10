use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, TransactionBehavior, params};

use crate::schema::open_initialized;
use crate::{Result, StoreError};

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS \
    codex_app_server_managed_targets (\
      thread_id TEXT PRIMARY KEY, \
      app_server_generation INTEGER NOT NULL, \
      created_at REAL NOT NULL, \
      updated_at REAL NOT NULL\
    )";

pub fn mark_app_server_managed_target(
    path: &Path,
    thread_id: &str,
    app_server_generation: i64,
) -> Result<()> {
    if thread_id.is_empty() || thread_id.trim() != thread_id {
        return Err(StoreError::InvalidAppServerManagedTarget(
            thread_id.to_owned(),
        ));
    }
    let observed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(StoreError::from)?
        .as_secs_f64();
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    mark_in_transaction(&transaction, thread_id, app_server_generation, observed_at)?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn mark_in_transaction(
    connection: &Connection,
    thread_id: &str,
    app_server_generation: i64,
    observed_at: f64,
) -> Result<()> {
    if thread_id.trim().is_empty() || thread_id.trim() != thread_id || app_server_generation <= 0 {
        return Err(StoreError::InvalidAppServerManagedTarget(thread_id.into()));
    }
    ensure_table(connection)?;
    connection.execute(
        "INSERT INTO codex_app_server_managed_targets \
         (thread_id, app_server_generation, created_at, updated_at) \
         VALUES (?, ?, ?, ?) ON CONFLICT(thread_id) DO UPDATE SET \
         app_server_generation = excluded.app_server_generation, \
         updated_at = excluded.updated_at",
        params![thread_id, app_server_generation, observed_at, observed_at],
    )?;
    Ok(())
}

pub(super) fn ensure_table(connection: &Connection) -> Result<()> {
    connection.execute(CREATE_TABLE, [])?;
    Ok(())
}

pub(super) fn contains(connection: &Connection, thread_id: &str) -> Result<bool> {
    ensure_table(connection)?;
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_app_server_managed_targets WHERE thread_id = ?)",
        [thread_id],
        |row| row.get(0),
    )?)
}
