//! Freeze the displayed prompt route separately from its turn-control binding.
use super::NewBusyChoice;
use crate::{Result, StoreError, mapping::mirrored_thread_id_in, schema::open_initialized};
use rusqlite::{Connection, TransactionBehavior, params};
use std::path::Path;

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('busy_choices') WHERE name='require_current_mirror')", [], |row| row.get(0))?)
}

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    if !schema_current(connection)? {
        // NULL deliberately marks old choices whose original route is unknown.
        connection.execute("ALTER TABLE busy_choices ADD COLUMN require_current_mirror INTEGER CHECK(require_current_mirror IN (0,1))", [])?;
    }
    Ok(())
}

pub fn create_busy_choice(path: &Path, choice: NewBusyChoice<'_>) -> Result<String> {
    create(path, choice, None)
}

pub fn create_busy_choice_on_route(
    path: &Path,
    choice: NewBusyChoice<'_>,
    mapped: bool,
) -> Result<String> {
    create(path, choice, Some(mapped))
}

fn create(path: &Path, choice: NewBusyChoice<'_>, expected: Option<bool>) -> Result<String> {
    let mut connection = open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = mirrored_thread_id_in(&tx, Some(choice.channel_id))?;
    let mapped = expected.unwrap_or(current.is_some());
    verify_route(&tx, choice.channel_id, choice.target_thread_id, mapped)?;
    tx.execute(
        "DELETE FROM busy_choices WHERE expires_at <= ?",
        [choice.now],
    )?;
    let id = uuid::Uuid::new_v4().simple().to_string()[..24].to_owned();
    tx.execute("INSERT INTO busy_choices (choice_id,owner_user_id,channel_id,target_thread_id,prompt,allow_steer,created_at,expires_at,claimed_at,require_current_mirror) VALUES (?,?,?,?,?,?,?,?,NULL,?)",
        params![id,choice.owner_user_id,choice.channel_id,choice.target_thread_id,choice.prompt,i64::from(choice.allow_steer),choice.now,choice.now+choice.time_to_live,i64::from(mapped)])?;
    tx.commit()?;
    Ok(id)
}

pub(crate) fn verify_route(
    connection: &Connection,
    channel: i64,
    target: Option<&str>,
    mapped: bool,
) -> Result<()> {
    let current = mirrored_thread_id_in(connection, Some(channel))?;
    if (mapped && (target.is_none() || current.as_deref() != target))
        || (!mapped && current.is_some())
    {
        return Err(StoreError::Integrity(
            "original busy prompt route changed; no request accepted".into(),
        ));
    }
    Ok(())
}
