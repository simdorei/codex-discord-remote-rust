//! Read-only restart-safety snapshot for the Rust Discord runtime.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::{Result, StoreError};

const READ_BUSY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestartReadinessSnapshot {
    pub target_thread_ids: BTreeSet<String>,
    pub blockers: Vec<String>,
    pub observations: Vec<String>,
}

/// Reads only state owned by the Rust bot. Missing or malformed core state is an error;
/// optional tables are absent only when that feature has never stored a row.
pub fn snapshot(path: &Path) -> Result<RestartReadinessSnapshot> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(READ_BUSY_TIMEOUT)?;

    let mut targets = BTreeSet::new();
    let mut blockers = Vec::new();
    let mut observations = Vec::new();
    read_mirror_targets(&connection, &mut targets)?;
    read_queue(&connection, &mut targets, &mut blockers, &mut observations)?;
    if table_exists(&connection, "codex_prompt_intakes")? {
        read_prompt_intakes(&connection, &mut targets, &mut blockers, &mut observations)?;
    }
    if table_exists(&connection, "codex_app_server_managed_targets")? {
        read_single_column(
            &connection,
            "SELECT thread_id FROM codex_app_server_managed_targets ORDER BY thread_id",
            &mut targets,
        )?;
    }
    if table_exists(&connection, "codex_thread_fork_handoffs")? {
        read_fork_targets(&connection, &mut targets, &mut observations)?;
    }
    blockers.sort();
    observations.sort();
    Ok(RestartReadinessSnapshot {
        target_thread_ids: targets,
        blockers,
        observations,
    })
}

fn read_fork_targets(
    connection: &Connection,
    targets: &mut BTreeSet<String>,
    observations: &mut Vec<String>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT handoff_id, source_thread_id, observed_target_thread_id, target_thread_id \
         FROM codex_thread_fork_handoffs ORDER BY handoff_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for row in rows {
        let (handoff, source, observed, target) = row?;
        for candidate in [
            Some(source.as_str()),
            observed.as_deref(),
            target.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_target(candidate)?;
            targets.insert(candidate.to_owned());
        }
        observations.push(format!(
            "fork:{handoff}:{source}:observed={}:target={}",
            observed.as_deref().unwrap_or("none"),
            target.as_deref().unwrap_or("none")
        ));
    }
    Ok(())
}

fn read_mirror_targets(connection: &Connection, targets: &mut BTreeSet<String>) -> Result<()> {
    read_single_column(
        connection,
        "SELECT codex_thread_id FROM mirror_threads ORDER BY codex_thread_id",
        targets,
    )
}

fn read_queue(
    connection: &Connection,
    targets: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
    observations: &mut Vec<String>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT job_id, target_thread_id, state, turn_id, last_error \
         FROM codex_turn_queue ORDER BY job_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (job_id, target, state, turn_id, last_error) = row?;
        validate_target(&target)?;
        targets.insert(target.clone());
        let semantic = match state.as_str() {
            "pending" => "pending",
            "starting" if last_error.starts_with(crate::queue::STARTING_CANDIDATE_HOLD_PREFIX) => {
                "starting_hold"
            }
            "starting" => {
                blockers.push(format!("queue job {job_id} is {state}"));
                "starting"
            }
            "running"
                if crate::queue::is_quarantine_encoding(
                    &state,
                    turn_id.as_deref(),
                    &last_error,
                ) =>
            {
                "quarantined"
            }
            "running" => {
                blockers.push(format!("queue job {job_id} is {state}"));
                "running"
            }
            other => return Err(StoreError::InvalidQueueState(other.to_owned())),
        };
        observations.push(format!("queue:{job_id}:{target}:{semantic}"));
    }
    Ok(())
}

fn read_prompt_intakes(
    connection: &Connection,
    targets: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
    observations: &mut Vec<String>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT job_id, target_thread_id, claim_token \
         FROM codex_prompt_intakes ORDER BY job_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (job_id, target, claim_token) = row?;
        validate_target(&target)?;
        targets.insert(target.clone());
        let claimed = claim_token.is_some();
        if claimed {
            blockers.push(format!("prompt intake {job_id} is claimed"));
        }
        observations.push(format!("intake:{job_id}:{target}:claimed={claimed}"));
    }
    Ok(())
}

fn read_single_column(
    connection: &Connection,
    query: &str,
    targets: &mut BTreeSet<String>,
) -> Result<()> {
    let mut statement = connection.prepare(query)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    for target in rows {
        let target = target?;
        validate_target(&target)?;
        targets.insert(target);
    }
    Ok(())
}

fn validate_target(target: &str) -> Result<()> {
    if target.is_empty() || target.trim() != target {
        return Err(StoreError::InvalidAppServerManagedTarget(target.to_owned()));
    }
    Ok(())
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
        [name],
        |row| row.get(0),
    )?)
}
