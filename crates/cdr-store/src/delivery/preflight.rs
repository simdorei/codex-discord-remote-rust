//! One short read snapshot for final-only preflight, never a send authorization.
use std::path::Path;

use rusqlite::Connection;

use super::StoredDelivery;
use crate::{Result, schema::checked_read::CheckedRead};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinalReadiness {
    Ready,
    Held(String),
    FirstReply(String),
    Commentary,
    GoalProgress,
}

/// The receipt writer still revalidates its exact identity and claim. No result
/// is published before this read snapshot finishes, and none is cached.
pub fn final_preflight(path: &Path, pending: &StoredDelivery) -> Result<FinalReadiness> {
    let snapshot = CheckedRead::open(path)?;
    snapshot.ensure_active()?;
    let readiness = read(snapshot.connection(), pending)?;
    snapshot.ensure_active()?;
    snapshot.finish()?;
    Ok(readiness)
}

pub(super) fn read(connection: &Connection, pending: &StoredDelivery) -> Result<FinalReadiness> {
    let explicit_grant = crate::final_recovery::authorized_in(connection, pending)?;
    if let Some(reason) = crate::new_reply::output_hold_in(connection, &pending.job_id)? {
        return Ok(FinalReadiness::Held(reason));
    }
    // Preserve only the existing saved-final exception. Its grant validation
    // includes original error evidence and an empty earlier-progress barrier.
    if !explicit_grant {
        if let Some(request) = crate::first_reply::pending_in(connection, &pending.job_id)? {
            return Ok(FinalReadiness::FirstReply(request));
        }
        if crate::commentary_outbox::has_pending_in(connection, &pending.job_id, None)? {
            return Ok(FinalReadiness::Commentary);
        }
    }
    if crate::goal_progress::has_pending_job_in(
        connection,
        &pending.job_id,
        &pending.target_thread_id,
    )? {
        return Ok(FinalReadiness::GoalProgress);
    }
    Ok(FinalReadiness::Ready)
}

#[cfg(test)]
mod tests;
