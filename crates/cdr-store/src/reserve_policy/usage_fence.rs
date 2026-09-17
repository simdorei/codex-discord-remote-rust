//! Durable evidence that a typed usage failure still needs policy handling.
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Claim {
    pub fence_id: i64,
    pub failure_policy_revision: Option<i64>,
    pub policy_revision: i64,
}

pub(crate) fn stage_in(connection: &Connection, thread_id: &str, reason: &str) -> Result<()> {
    let mode: Option<String> = connection
        .query_row(
            "SELECT mode FROM codex_reserve_policy WHERE thread_id=?1",
            [thread_id],
            |row| row.get(0),
        )
        .optional()?;
    match mode.as_deref() {
        Some("auto" | "on") => {}
        Some("manual" | "off") => return Ok(()),
        Some(other) => {
            return Err(StoreError::Integrity(format!(
                "Unsupported Reserve policy mode: {other}"
            )));
        }
        None => {
            return Err(StoreError::Integrity(
                "Reserve policy is missing for usage failure".into(),
            ));
        }
    }

    let updated = connection.execute(
        "UPDATE codex_reserve_policy
         SET usage_failure_state='pending', usage_failure_revision=revision,
             usage_failure_id=COALESCE(usage_failure_id, 0) + 1,
             usage_failure_reason=?2, usage_failure_resolution_reason=NULL,
             usage_failure_updated_at=unixepoch()
         WHERE thread_id=?1 AND mode IN ('auto','on')",
        params![thread_id, reason],
    )?;
    if updated != 1 {
        return Err(StoreError::Integrity(
            "Cannot stage usage failure without automatic policy".into(),
        ));
    }
    Ok(())
}

pub(crate) fn claim_in(connection: &Connection, thread_id: &str) -> Result<Option<Claim>> {
    connection
        .query_row(
            "SELECT usage_failure_id, usage_failure_revision, revision
             FROM codex_reserve_policy
             WHERE thread_id=?1 AND usage_failure_state='pending'",
            [thread_id],
            |row| {
                Ok(Claim {
                    fence_id: row.get(0)?,
                    failure_policy_revision: row.get(1)?,
                    policy_revision: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn resolve_in(
    connection: &Connection,
    thread_id: &str,
    expected_fence_id: i64,
    expected_failure_policy_revision: i64,
    expected_current_policy_revision: i64,
    reason: &str,
) -> Result<bool> {
    Ok(connection.execute(
        "UPDATE codex_reserve_policy
         SET usage_failure_state='resolved', usage_failure_resolution_reason=?5,
             usage_failure_updated_at=unixepoch()
         WHERE thread_id=?1 AND usage_failure_state='pending'
            AND usage_failure_id=?2 AND usage_failure_revision=?3
            AND revision=?4 AND mode IN ('auto','on')",
        params![
            thread_id,
            expected_fence_id,
            expected_failure_policy_revision,
            expected_current_policy_revision,
            reason
        ],
    )? == 1)
}

pub(crate) fn resolve_episode_in(
    connection: &Connection,
    thread_id: &str,
    claim: Option<Claim>,
    expected_current_policy_revision: i64,
    reason: &str,
) -> Result<bool> {
    let Some(claim) = claim else {
        return Ok(false);
    };
    let Some(failure_policy_revision) = claim.failure_policy_revision else {
        return Ok(false);
    };
    Ok(connection.execute(
        "UPDATE codex_reserve_policy
         SET usage_failure_state='resolved', usage_failure_resolution_reason=?5,
             usage_failure_updated_at=unixepoch()
         WHERE thread_id=?1 AND usage_failure_state='pending'
           AND usage_failure_id=?2 AND usage_failure_revision=?3
           AND revision=?4
           AND mode IN ('auto','on')",
        params![
            thread_id,
            claim.fence_id,
            failure_policy_revision,
            expected_current_policy_revision,
            reason
        ],
    )? == 1)
}

pub(crate) fn resolve_manual_in(connection: &Connection, thread_id: &str) -> Result<()> {
    connection.execute(
        "UPDATE codex_reserve_policy
         SET usage_failure_state='resolved',
             usage_failure_resolution_reason='manual policy override',
             usage_failure_updated_at=unixepoch()
         WHERE thread_id=?1 AND usage_failure_state='pending'",
        [thread_id],
    )?;
    Ok(())
}

pub(crate) fn unresolved(path: &Path, thread_id: &str) -> Result<bool> {
    Ok(open_initialized(path)?.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_reserve_policy
         WHERE thread_id=?1 AND usage_failure_state='pending')",
        [thread_id],
        |row| row.get(0),
    )?)
}
