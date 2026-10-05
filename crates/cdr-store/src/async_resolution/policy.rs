//! Reviewed incident safety policy, never a publishing or replay authorization.
use super::{FORMAT_VERSION, has_table, held};
use crate::Result;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;

pub const REVIEWED_INCIDENT_THREAD: &str = "01a06156-56cd-70b0-af02-2de7445ba4c7";
pub const RECOVERY_POLICY_FORMAT_VERSION: i64 = 1;
pub const RECOVERY_POLICY_COMPONENT: &str = "async_recovery_policy";
pub const REVIEWED_PROPOSAL_SHA256: &str =
    "b1ffb41351d1086c0d807cd900c0fc651ec3577a9455af18dc6d11ec77218c95";
const ORIGINAL_TURN: &str = "01a0d9f0-8cb2-7e71-a0c1-2644d8ae45db";
const ORIGINAL_JOB: &str = "0cd97817-5f74-4e39-a74d-68ed9259c5a5";
const REVIEWED_PENDING: &str = "b3d5a1a3-5c3e-4764-967b-0cef767efde9";
type Registration = (i64, String, String, String, String, String);

fn expected() -> Registration {
    (
        RECOVERY_POLICY_FORMAT_VERSION,
        "publishing_recovery".into(),
        REVIEWED_PROPOSAL_SHA256.into(),
        ORIGINAL_TURN.into(),
        ORIGINAL_JOB.into(),
        REVIEWED_PENDING.into(),
    )
}

fn registration_in(db: &Connection) -> Result<Option<Registration>> {
    Ok(db
        .query_row(
            "SELECT format_version,policy,proposal_sha256,original_turn_id,
        origin_job_id,pending_job_id FROM cdr_async_recovery_policies WHERE thread_id=?",
            [REVIEWED_INCIDENT_THREAD],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?)
}

pub fn reviewed_policy_installed_in(db: &Connection) -> Result<bool> {
    if !has_table(db, "cdr_async_recovery_policies")? {
        return Ok(false);
    }
    Ok(registration_in(db)?.is_some_and(|row| row == expected()))
}

pub(super) fn held_in(db: &Connection, thread: &str) -> Result<bool> {
    // Missing/failed policy installation is never clean-ordinary permission.
    // This exact reviewed incident fallback does not seal unrelated targets.
    if thread == REVIEWED_INCIDENT_THREAD {
        return Ok(true);
    }
    if !has_table(db, "cdr_async_recovery_policies")? {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_recovery_policies WHERE thread_id=?)",
        [thread],
        |r| r.get(0),
    )?)
}

pub(super) fn capture_case(thread_column: &str) -> String {
    format!(
        "CASE WHEN {thread_column}='{REVIEWED_INCIDENT_THREAD}' OR EXISTS(
        SELECT 1 FROM cdr_async_recovery_policies p WHERE p.thread_id={thread_column})
        THEN 'publishing_recovery' ELSE 'ordinary' END"
    )
}

/// Called by owning-runtime bootstrap under its shared target lock/control
/// permit. The identities describe a safety hold, not an approved pending input.
pub fn install_reviewed_policy(path: &Path) -> Result<()> {
    let mut db = crate::schema::open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = tx.query_row(
        "SELECT format_version FROM cdr_runtime_capability_requirements WHERE component=?",
        [RECOVERY_POLICY_COMPONENT],
        |r| r.get(0),
    )?;
    if version != RECOVERY_POLICY_FORMAT_VERSION {
        return Err(held(
            REVIEWED_INCIDENT_THREAD,
            "unsupported persisted recovery-policy capability",
        ));
    }
    if let Some(existing) = registration_in(&tx)? {
        if existing != expected() {
            return Err(held(
                REVIEWED_INCIDENT_THREAD,
                "reviewed incident policy identity changed; existing evidence preserved",
            ));
        }
    } else {
        tx.execute("INSERT INTO cdr_async_recovery_policies
            (thread_id,format_version,policy,proposal_sha256,original_turn_id,origin_job_id,pending_job_id)
            VALUES(?,?,'publishing_recovery',?,?,?,?)",
            params![REVIEWED_INCIDENT_THREAD,RECOVERY_POLICY_FORMAT_VERSION,REVIEWED_PROPOSAL_SHA256,
                ORIGINAL_TURN,ORIGINAL_JOB,REVIEWED_PENDING])?;
    }
    let unsupported: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_execution_obligations WHERE thread_id=?
         AND (format_version!=? OR policy NOT IN ('ordinary','publishing_recovery')))",
        params![REVIEWED_INCIDENT_THREAD, FORMAT_VERSION],
        |r| r.get(0),
    )?;
    if unsupported {
        return Err(held(
            REVIEWED_INCIDENT_THREAD,
            "unsupported original obligation policy; no evidence rewritten",
        ));
    }
    tx.execute("UPDATE cdr_async_execution_obligations SET policy='publishing_recovery',admission_state='held'
        WHERE thread_id=? AND policy='ordinary'",[REVIEWED_INCIDENT_THREAD])?;
    let remaining: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_execution_obligations WHERE thread_id=?
         AND (policy!='publishing_recovery' OR admission_state!='held'))",
        [REVIEWED_INCIDENT_THREAD],
        |r| r.get(0),
    )?;
    if remaining || !reviewed_policy_installed_in(&tx)? {
        return Err(held(
            REVIEWED_INCIDENT_THREAD,
            "reviewed incident hold did not commit exactly",
        ));
    }
    tx.commit()?;
    Ok(())
}
