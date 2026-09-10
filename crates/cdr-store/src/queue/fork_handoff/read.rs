use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, TransactionBehavior};

use super::{AppServerForkHandoff, storage};
use crate::schema::open_initialized;
use crate::{Result as StoreResult, StoreError};

pub(in crate::queue) fn ensure_schema(connection: &Connection) -> StoreResult<()> {
    storage::ensure_table(connection)?;
    Ok(())
}

pub(super) fn unresolved_in_connection(
    connection: &Connection,
    source_thread_id: &str,
) -> StoreResult<Option<AppServerForkHandoff>> {
    storage::ensure_table(connection)?;
    Ok(storage::by_source(connection, source_thread_id)?
        .filter(|handoff| handoff.target_thread_id.is_none()))
}

pub(in crate::queue) fn ensure_no_unresolved_handoff(
    connection: &Connection,
    target_thread_id: &str,
) -> StoreResult<()> {
    if let Some(handoff) = unresolved_in_connection(connection, target_thread_id)? {
        let last_error =
            (!handoff.last_fork_error.trim().is_empty()).then_some(handoff.last_fork_error);
        return Err(StoreError::ForkHandoffUnresolved {
            target_thread_id: target_thread_id.to_owned(),
            last_error,
        });
    }
    Ok(())
}

pub(in crate::queue) fn ensure_source_not_moved(
    connection: &Connection,
    source_thread_id: &str,
) -> StoreResult<()> {
    if super::retirement::enabled(connection)? {
        return Ok(());
    }
    storage::ensure_table(connection)?;
    if let Some(target_thread_id) =
        storage::completed_target_for_source(connection, source_thread_id)?
    {
        return Err(StoreError::ForkHandoffTargetMoved {
            source_thread_id: source_thread_id.to_owned(),
            target_thread_id,
        });
    }
    Ok(())
}

pub(crate) fn canonical_completed_target(
    connection: &Connection,
    source_thread_id: &str,
) -> StoreResult<String> {
    if super::retirement::enabled(connection)? {
        crate::dead_generation::ensure_target_available(connection, source_thread_id)?;
        return Ok(source_thread_id.to_owned());
    }
    storage::ensure_table(connection)?;
    let mut current = source_thread_id.to_owned();
    let mut visited = BTreeSet::new();
    while visited.insert(current.clone()) {
        crate::dead_generation::ensure_target_available(connection, &current)?;
        let Some(target) = storage::completed_target_for_source(connection, &current)? else {
            return Ok(current);
        };
        current = target;
    }
    Err(StoreError::ForkHandoffCycle(source_thread_id.to_owned()))
}

pub fn unresolved_app_server_fork_handoff_for_source(
    path: &Path,
    source_thread_id: &str,
) -> StoreResult<Option<AppServerForkHandoff>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let handoff = unresolved_in_connection(&transaction, source_thread_id)?;
    transaction.commit()?;
    Ok(handoff)
}

pub fn is_app_server_managed_target(path: &Path, thread_id: &str) -> StoreResult<bool> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_table(&transaction)?;
    let managed = storage::managed_target(&transaction, thread_id)?
        || super::super::managed_target::contains(&transaction, thread_id)?;
    transaction.commit()?;
    Ok(managed)
}

pub fn completed_app_server_fork_target_for_source(
    path: &Path,
    source_thread_id: &str,
) -> StoreResult<Option<String>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::dead_generation::ensure_target_available(&transaction, source_thread_id)?;
    storage::ensure_table(&transaction)?;
    let target = storage::completed_target_for_source(&transaction, source_thread_id)?;
    transaction.commit()?;
    Ok(target)
}
