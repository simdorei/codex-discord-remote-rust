//! A terminal repair does not dispose of a later, unprocessed lifecycle request.
//! Admission only: this never authorizes, dispatches, acknowledges or clears one.
use super::{MAX_EVIDENCE_BYTES, MAX_TARGET_RECORDS, has_table};
use crate::Result;
use rusqlite::{Connection, params};
use serde_json::Value;
mod classification;

pub(super) fn admission_held_in(db: &Connection, thread: &str) -> Result<bool> {
    let repaired: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_execution_obligations WHERE thread_id=?)",
        [thread],
        |row| row.get(0),
    )?;
    if !repaired || !has_table(db, "discord_ingress_journal")? {
        return Ok(false);
    }
    let mut statement = db.prepare(
        "SELECT CASE WHEN length(CAST(payload_json AS BLOB))<=?2 THEN payload_json END,
         CASE WHEN length(CAST(outcome_json AS BLOB))<=?2 THEN outcome_json END
         FROM discord_ingress_journal
         WHERE target_thread_id=?1 AND owner_id IS NULL AND state!='completed'
         AND NOT(phase IN ('result_recorded','stop_accepted') AND outcome_json IS NOT NULL)
         ORDER BY created_at,ingress_id LIMIT 129",
    )?;
    let byte_limit = i64::try_from(MAX_EVIDENCE_BYTES)
        .map_err(|_| super::held(thread, "unsupported lifecycle evidence bound"))?;
    let mut rows = statement.query(params![thread, byte_limit])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        if count > MAX_TARGET_RECORDS {
            return Ok(true);
        }
        let encoded: Option<String> = row.get(0)?;
        let Some(encoded) = encoded else {
            return Ok(true);
        };
        let Ok(payload) = serde_json::from_str::<Value>(&encoded) else {
            return Ok(true);
        };
        if !payload.is_object() {
            return Ok(true);
        }
        let declared = control(payload.pointer("/plan/Execute"))
            || control(payload.pointer("/lifecycle_binding/command"))
            || matches!(
                payload.pointer("/work/Slash/name").and_then(Value::as_str),
                Some("stop" | "archive")
            )
            || matches!(
                payload.get("command").and_then(Value::as_str),
                Some("stop" | "archive")
            );
        if !declared && classification::ordinary(&payload) {
            continue;
        }
        let outcome: Option<String> = row.get(1)?;
        let outcome = outcome
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok());
        // A recorded pre-effect refusal has already disposed of the action;
        // uncertain notification delivery must not revive it as an execution hold.
        if crate::ingress::CleanupRefusal::from_outcome(outcome.as_ref()).is_some() {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

fn control(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        matches!(value.as_str(), Some("Stop" | "Archive"))
            || value
                .as_object()
                .is_some_and(|fields| fields.contains_key("Stop") || fields.contains_key("Archive"))
    })
}
