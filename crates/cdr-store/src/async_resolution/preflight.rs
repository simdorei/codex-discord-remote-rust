//! Bounded negative evidence only; no original execution can be authorized here.
use rusqlite::{Connection, params};

use super::{Obligation, history::original_question, read_in};
use crate::{Result, StoreError};

const ROWS: usize = 128;
const BYTES: usize = 2 * 1024 * 1024;
const SIZES: &str = "SELECT COUNT(*),COALESCE(SUM(bytes),0) FROM (
    SELECT COALESCE(length(CAST(question_id AS BLOB)),0)
      +COALESCE(length(CAST(thread_id AS BLOB)),0)
      +COALESCE(length(CAST(origin_job_id AS BLOB)),0)
      +COALESCE(length(CAST(turn_id AS BLOB)),0)
      +COALESCE(length(CAST(answer_state AS BLOB)),0)
      +COALESCE(length(CAST(execution_state AS BLOB)),0)
      +COALESCE(length(CAST(admission_state AS BLOB)),0)
      +COALESCE(length(CAST(policy AS BLOB)),0)
      +COALESCE(length(CAST(original_seal AS BLOB)),0)
      +COALESCE(length(CAST(claim_json AS BLOB)),0)
      +COALESCE(length(CAST(owner_json AS BLOB)),0)
      +COALESCE(length(CAST(original_error AS BLOB)),0) AS bytes
    FROM cdr_async_unsettled_obligations WHERE thread_id=?1
    ORDER BY question_id LIMIT ?2)";

fn refused(row: &Obligation) -> Result<bool> {
    let Err(error) = original_question(row) else {
        return Ok(false);
    };
    // Only errors produced by this pure stored-evidence validator are negatives.
    // A future SQL/I/O/other failure must propagate, never become a skip hint.
    if matches!(
        &error,
        StoreError::AsyncResolutionHeld { .. } | StoreError::Json(_)
    ) || matches!(&error, StoreError::Integrity(reason) if reason == "invalid sealed answer option")
    {
        Ok(true)
    } else {
        Err(error)
    }
}

/// The caller owns one checked read snapshot. Output positions refer to inputs,
/// not permissions. All strings are size-checked before `read_in` materializes them.
pub(crate) fn unprovable_in(db: &Connection, targets: &[&str]) -> Result<Vec<usize>> {
    let mut remaining_rows = ROWS;
    let mut remaining_bytes = BYTES;
    let mut negative = Vec::new();
    let mut sizes = db.prepare(SIZES)?;
    for (index, target) in targets.iter().enumerate() {
        if remaining_rows == 0 {
            break;
        }
        let limit = i64::try_from(remaining_rows + 1).map_err(|_| {
            StoreError::Integrity("orphan preflight row limit is out of range".into())
        })?;
        let (count, bytes): (i64, i64) =
            sizes.query_row(params![target, limit], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let count = usize::try_from(count).map_err(|_| {
            StoreError::Integrity("orphan preflight row count is out of range".into())
        })?;
        let bytes = usize::try_from(bytes).map_err(|_| {
            StoreError::Integrity("orphan preflight byte count is out of range".into())
        })?;
        if count > remaining_rows {
            // The single overflow probe contains scalars, not claim/seal bodies.
            remaining_rows = 0;
            continue;
        }
        remaining_rows -= count;
        if count == 0 || bytes > remaining_bytes {
            continue;
        }
        let rows = read_in(db, target)?;
        if rows.len() != count {
            return Err(StoreError::Integrity(
                "orphan preflight evidence set changed inside its snapshot".into(),
            ));
        }
        remaining_bytes -= bytes;
        #[cfg(test)]
        crate::schema::checked_read::test_support::boundary(
            crate::schema::checked_read::test_support::Boundary::OrphanMaterialized(count, bytes),
            db,
        )?;
        for row in &rows {
            if refused(row)? {
                negative.push(index);
                break;
            }
        }
    }
    Ok(negative)
}
