use std::path::Path;

use rusqlite::params;

use super::{StoredPromptIntake, storage};
use crate::Result;
use crate::schema::open_initialized;

pub fn get_prompt_intake(path: &Path, job_id: &str) -> Result<Option<StoredPromptIntake>> {
    let connection = open_initialized(path)?;
    storage::ensure_schema(&connection)?;
    storage::by_job(&connection, job_id)
}

pub fn list_prompt_intakes(path: &Path) -> Result<Vec<StoredPromptIntake>> {
    let connection = open_initialized(path)?;
    storage::ensure_schema(&connection)?;
    storage::list(&connection, None)
}

pub fn list_ready_prompt_intakes(path: &Path, now: f64) -> Result<Vec<StoredPromptIntake>> {
    let connection = open_initialized(path)?;
    storage::ensure_schema(&connection)?;
    storage::list(&connection, Some(now))
}

pub fn prompt_intake_has_durable_owner(path: &Path, job_id: &str) -> Result<bool> {
    let connection = open_initialized(path)?;
    storage::ensure_schema(&connection)?;
    Ok(connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM codex_prompt_intakes WHERE job_id = ?) \
         AND (EXISTS(SELECT 1 FROM codex_turn_queue WHERE job_id = ?) \
              OR EXISTS(SELECT 1 FROM codex_delivery_outbox WHERE job_id = ?))",
        params![job_id, job_id, job_id],
        |row| row.get(0),
    )?)
}
