//! Durable receipts and manual holds for work whose app-server process died.

mod capture;
mod schema;

use std::path::Path;

use rusqlite::{Connection, params};

use crate::{Result, StoreError};

pub use capture::{DeadGenerationCapture, capture_dead_generation};
pub(crate) use schema::{migrate_schema, schema_current};

pub fn activate_runtime(path: &Path, runtime_id: &str) -> Result<()> {
    if runtime_id.trim().is_empty() {
        return Err(StoreError::Integrity(
            "empty app-server runtime identity".into(),
        ));
    }
    crate::schema::open_initialized(path)?.execute(
        "INSERT INTO codex_app_server_runtime (singleton, runtime_id) VALUES (1, ?) \
         ON CONFLICT(singleton) DO UPDATE SET runtime_id = excluded.runtime_id",
        [runtime_id],
    )?;
    Ok(())
}

pub fn target_is_held(path: &Path, target: &str) -> Result<bool> {
    target_is_held_in(&crate::schema::open_initialized(path)?, target)
}

pub(crate) fn target_is_held_in(connection: &Connection, target: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_dead_generation_holds WHERE target_thread_id = ?)",
        [target],
        |row| row.get(0),
    )?)
}

pub(crate) fn ensure_target_available(connection: &Connection, target: &str) -> Result<()> {
    if target_is_held_in(connection, target)? {
        return Err(StoreError::DeadGenerationTargetHeld(target.to_owned()));
    }
    Ok(())
}

pub fn generation_is_sealed(path: &Path, generation: i64) -> Result<bool> {
    generation_is_sealed_in(&crate::schema::open_initialized(path)?, generation)
}

pub(crate) fn generation_is_sealed_in(connection: &Connection, generation: i64) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_dead_generation_incidents incident \
         JOIN codex_app_server_runtime runtime ON runtime.runtime_id = incident.runtime_id \
         WHERE runtime.singleton = 1 AND incident.generation = ?)",
        params![generation],
        |row| row.get(0),
    )?)
}

pub(crate) fn job_can_mutate(
    connection: &Connection,
    job: &crate::queue::StoredQueueJob,
) -> Result<bool> {
    Ok(!target_is_held_in(connection, &job.target_thread_id)?
        && !generation_is_sealed_in(connection, job.app_server_generation)?)
}
