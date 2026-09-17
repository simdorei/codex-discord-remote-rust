//! Durable intent for changing effort on an already-Reserve thread.
//! Reuse the existing entering hold; a crash never authorizes settings replay.
use super::{EpisodeIdentity, Policy, get_connection, usage_fence};
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{TransactionBehavior, params};
use std::path::Path;

#[derive(Debug)]
pub struct AlignmentClaim {
    expected: Policy,
    previous_state: String,
    usage_failure: Option<usage_fence::Claim>,
}

impl AlignmentClaim {
    #[must_use]
    pub const fn revision(&self) -> i64 {
        self.expected.revision
    }
}

/// Capture exact policy and the current usage fence before any settings dispatch.
/// Existing restore settings are never replaced with the intermediate Reserve settings.
pub fn begin(
    path: &Path,
    expected: &Policy,
    identity: EpisodeIdentity<'_>,
    effort: &str,
) -> Result<Option<AlignmentClaim>> {
    if !matches!(expected.mode.as_str(), "auto" | "on")
        || !matches!(expected.state.as_str(), "ordinary" | "reserve")
        || !matches!(effort, "medium" | "high" | "xhigh")
        || identity.process_id.is_none()
        || identity.generation < 0
        || identity.account_id.trim().is_empty()
        || (expected.state == "reserve"
            && (expected.account_id.as_deref() != Some(identity.account_id)
                || expected.applied_model.as_deref() != Some("gpt-reserve")))
    {
        return Ok(None);
    }
    let mut connection = open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if get_connection(&tx, &expected.thread_id)?.as_ref() != Some(expected) {
        return Ok(None);
    }
    crate::dead_generation::ensure_target_available(&tx, &expected.thread_id)?;
    let usage_failure = usage_fence::claim_in(&tx, &expected.thread_id)?;
    let changed = tx.execute(
        "UPDATE codex_reserve_policy SET state='entering', revision=revision+1,
         account_id=?2,process_id=?3,generation=?4,applied_model='gpt-reserve',
         applied_effort=?5,applied_tier='default',updated_at=unixepoch()
         WHERE thread_id=?1 AND revision=?6 AND state=?7 AND mode IN ('auto','on')
         AND NOT EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)",
        params![
            expected.thread_id,
            identity.account_id,
            identity.process_id,
            identity.generation,
            effort,
            expected.revision,
            expected.state
        ],
    )?;
    if changed != 1 {
        return Ok(None);
    }
    let claimed = get_connection(&tx, &expected.thread_id)?
        .ok_or_else(|| StoreError::Integrity("Reserve effort intent was not observable".into()))?;
    tx.commit()?;
    Ok(Some(AlignmentClaim {
        expected: claimed,
        previous_state: expected.state.clone(),
        usage_failure,
    }))
}

/// Commit only after exact settings and the final account/model response validate.
/// No new transition notice: this adjusts effort, not ordinary/Reserve routing.
pub fn finish(path: &Path, claim: &AlignmentClaim) -> Result<bool> {
    let expected = &claim.expected;
    let mut connection = open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if get_connection(&tx, &expected.thread_id)?.as_ref() != Some(expected) {
        return Ok(false);
    }
    crate::dead_generation::ensure_target_available(&tx, &expected.thread_id)?;
    let changed = tx.execute(
        "UPDATE codex_reserve_policy SET state=?2, revision=revision+1,updated_at=unixepoch()
         WHERE thread_id=?1 AND revision=?3 AND state='entering' AND mode IN ('auto','on')
         AND NOT EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)",
        params![expected.thread_id, claim.previous_state, expected.revision],
    )?;
    if changed != 1 {
        return Ok(false);
    }
    usage_fence::resolve_episode_in(
        &tx,
        &expected.thread_id,
        claim.usage_failure,
        expected.revision + 1,
        "Reserve effort alignment and final capacity confirmed",
    )?;
    // A newer fence or legacy NULL revision must roll back the success as well.
    if usage_fence::claim_in(&tx, &expected.thread_id)?.is_some() {
        return Ok(false);
    }
    tx.commit()?;
    Ok(true)
}
