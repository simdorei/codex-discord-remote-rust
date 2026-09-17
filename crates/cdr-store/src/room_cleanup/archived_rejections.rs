//! Narrow archived-room reconciliation. Ordinary cleanup remains strict.
//! Notification delivery is never inferred from a definite control non-dispatch.
use std::path::Path;

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use serde_json::Value;

use crate::{Result, claims::BusyChoice};
mod evidence;
pub(super) use evidence::{migrate, schema_current};

pub fn pending_reason(
    path: &Path,
    channel: i64,
    target: &str,
    now: f64,
) -> Result<Option<&'static str>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let transaction = connection.unchecked_transaction()?;
    let allowed = eligible(&transaction, channel, target, now)?;
    let ids = allowed.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
    super::pending::reason_with_exclusions(&transaction, channel, Some(target), None, false, &ids)
}

/// The caller must pin the remote room identity and freshly validate its archived
/// source in this callback. Errors roll back BOTH the fence and audit snapshots.
pub fn begin(
    path: &Path,
    channel: i64,
    target: &str,
    parent: i64,
    now: f64,
    verify_archive: impl FnOnce() -> Result<String>,
) -> Result<String> {
    let mut connection = crate::schema::open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let matches: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM mirror_threads WHERE codex_thread_id=? AND discord_thread_id=? AND discord_channel_id=?)",
        params![target, channel, parent], |row| row.get(0),
    )?;
    if parent <= 0 || !matches {
        return Err(crate::StoreError::Integrity(
            "archived cleanup parent identity changed".into(),
        ));
    }
    let allowed = eligible(&transaction, channel, target, now)?;
    let ids = allowed.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
    let token = super::begin_in(&transaction, channel, Some(target), now, &ids)?;
    let archive = verify_archive()?;
    // A malformed callback's evidence must never create a successful audit.
    let _: Value = serde_json::from_str(&archive)?;
    for row in allowed {
        transaction.execute(
            "INSERT INTO cdr_archived_cleanup_evidence
             (token,channel_id,target_thread_id,ingress_id,payload_json,outcome_json,row_snapshot_json,archive_json,created_at)
             VALUES (?,?,?,?,?,?,?,?,?)",
            params![token,channel,target,row.id,row.payload,row.outcome,row.snapshot,archive,now],
        )?;
    }
    transaction.commit()?;
    Ok(token)
}

/// Exact audit plus an extant close fence stops recovery from staging a new POST
/// for an already reconciled held row. A rejected/released intent cannot do so.
pub(crate) fn preserves(connection: &Connection, ingress_id: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_archived_cleanup_evidence e
         JOIN cdr_cleanup_fences f ON f.token=e.token AND f.channel_id=e.channel_id
             AND f.target_thread_id=e.target_thread_id
         JOIN discord_ingress_journal j ON j.ingress_id=e.ingress_id
             AND j.channel_id=e.channel_id AND j.target_thread_id=e.target_thread_id
             AND j.payload_json=e.payload_json AND j.outcome_json=e.outcome_json
         WHERE e.ingress_id=?
             AND j.owner_user_id=json_extract(e.row_snapshot_json,'$.owner_user_id')
             AND j.canonical_owner IS json_extract(e.row_snapshot_json,'$.canonical_owner')
             AND j.event_id IS json_extract(e.row_snapshot_json,'$.event_id')
             AND j.runtime_id IS json_extract(e.row_snapshot_json,'$.runtime_id')
             AND j.hold_reason=json_extract(e.row_snapshot_json,'$.hold_reason')
             AND j.updated_at=json_extract(e.row_snapshot_json,'$.updated_at')
             AND j.state='held' AND j.phase='result_recorded'
             AND j.owner_kind IS NULL AND j.owner_id IS NULL AND j.confirmation_delivered=0)",
        [ingress_id],
        |row| row.get(0),
    )?)
}

struct Rejection {
    id: String,
    payload: String,
    outcome: String,
    actor: i64,
    canonical: String,
    snapshot: String,
}

fn eligible(
    connection: &Connection,
    channel: i64,
    target: &str,
    now: f64,
) -> Result<Vec<Rejection>> {
    let mut statement = connection.prepare(evidence::CANDIDATES)?;
    let rows = statement.query_map(params![channel, target], |row| {
        Ok(Rejection {
            id: row.get(0)?,
            payload: row.get(1)?,
            outcome: row.get(2)?,
            actor: row.get(3)?,
            canonical: row.get(4)?,
            snapshot: row.get(5)?,
        })
    })?;
    let mut allowed = Vec::new();
    for row in rows {
        let row = row?;
        if proves_rejection(&row, channel, target, now) {
            allowed.push(row);
        }
    }
    Ok(allowed)
}

fn proves_rejection(row: &Rejection, channel: i64, target: &str, now: f64) -> bool {
    let Ok(outcome) = serde_json::from_str::<Value>(&row.outcome) else {
        return false;
    };
    if outcome.get("kind").and_then(Value::as_str) != Some("busy_control_preflight_rejected")
        || outcome.get("control_dispatched").and_then(Value::as_bool) != Some(false)
    {
        return false;
    }
    let Ok(payload) = serde_json::from_str::<Value>(&row.payload) else {
        return false;
    };
    if payload.get("busy_action").and_then(Value::as_str) != Some("steer") {
        return false;
    }
    let Ok(choice) = serde_json::from_value::<BusyChoice>(payload["busy_choice"].clone()) else {
        return false;
    };
    !choice.choice_id.trim().is_empty()
        && row.canonical == format!("busy-choice:{}", choice.choice_id)
        && row.actor > 0
        && choice.owner_user_id == row.actor
        && choice.channel_id == channel
        && choice.target_thread_id.as_deref() == Some(target)
        && !choice.allow_steer
        && now.is_finite()
        && choice.created_at.is_finite()
        && choice.expires_at.is_finite()
        && choice.expires_at >= choice.created_at
        && choice.expires_at <= now
}
