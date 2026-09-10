use super::{new_command_prompt, read::get_in};
use crate::{Result, StoreError};
use rusqlite::{Connection, params};

/// Validate every original owner without rewriting its channel to the new room.
pub(crate) fn cancellation_owners(
    db: &Connection,
    job: &str,
    event: Option<i64>,
    target: &str,
    channel: i64,
    owner: i64,
) -> Result<Vec<String>> {
    let keys = db
        .prepare(
            "SELECT ingress_id FROM discord_ingress_journal
        WHERE owner_id=?1 OR (?2 IS NOT NULL AND event_id=?2)",
        )?
        .query_map(params![job, event], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for key in &keys {
        let row = get_in(db, key)?.ok_or_else(|| invalid(key))?;
        if row.owner_kind.as_deref() != Some("prompt")
            || row.owner_id.as_deref() != Some(job)
            || row.owner_user_id != owner
            || row.target_thread_id.as_deref() != Some(target)
            || !matches!(row.state.as_str(), "owned" | "completed")
        {
            return Err(invalid(key));
        }
        if row.channel_id == channel {
            continue;
        }
        let creation = row.outcome.as_ref().and_then(|v| v.get("new_creation"));
        let verified_new_room = new_command_prompt(&row).is_some()
            && creation
                .and_then(|v| v.get("version"))
                .and_then(serde_json::Value::as_i64)
                == Some(1)
            && creation
                .and_then(|v| v.get("origin_channel_id"))
                .and_then(serde_json::Value::as_i64)
                == Some(row.channel_id)
            && crate::mapping::mirrored_thread_id_in(db, Some(channel))?.as_deref() == Some(target);
        if !verified_new_room {
            return Err(invalid(key));
        }
    }
    Ok(keys)
}

fn invalid(key: &str) -> StoreError {
    StoreError::Integrity(format!(
        "request {key} has conflicting or uncertain cancellation ownership; nothing was cancelled"
    ))
}
