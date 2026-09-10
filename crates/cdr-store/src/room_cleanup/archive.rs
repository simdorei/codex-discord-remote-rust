//! Permanent local transcript deletion guard, independent of Discord room deletion.
use crate::{Result, StoreError};
use rusqlite::{TransactionBehavior, params};
use std::path::Path;

pub fn begin(path: &Path, target: &str, now: f64) -> Result<String> {
    begin_confirmed(path, target, now, None)
}

pub struct Confirmation<'a> {
    pub message_id: i64,
    pub channel_id: i64,
    pub user_id: i64,
    pub reference: &'a str,
}

pub fn begin_confirmed(
    path: &Path,
    target: &str,
    now: f64,
    confirmation: Option<&Confirmation<'_>>,
) -> Result<String> {
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if target.is_empty() || !now.is_finite() {
        return Err(StoreError::Integrity(
            "invalid archive deletion identity".into(),
        ));
    }
    let confirmation_id = confirmation.map(|c| format!("message:{}", c.message_id));
    if let Some(c) = confirmation {
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM discord_ingress_journal WHERE ingress_id=?1 AND kind='message' AND event_id=?2 AND channel_id=?3 AND owner_user_id=?4 AND state='executing' AND json_extract(payload_json,'$.plan.Execute.DeleteArchiveConfirm.reference')=?5)",params![confirmation_id,c.message_id,c.channel_id,c.user_id,c.reference],|r|r.get(0))?;
        if !valid {
            return Err(StoreError::Integrity(
                "archive confirmation custody identity changed".into(),
            ));
        }
    }
    let rooms = tx
        .prepare("SELECT discord_thread_id FROM mirror_threads WHERE codex_thread_id=?")?
        .query_map([target], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for channel in std::iter::once(0).chain(rooms.iter().copied()) {
        if let Some(reason) =
            super::pending::reason_except(&tx, channel, Some(target), confirmation_id.as_deref())?
        {
            return Err(StoreError::Integrity(format!(
                "archive deletion protected by {reason}"
            )));
        }
    }
    let token = uuid::Uuid::new_v4().to_string();
    let channels =
        serde_json::to_string(&rooms).map_err(|e| StoreError::Integrity(e.to_string()))?;
    tx.execute("INSERT INTO cdr_archive_fences(target_thread_id,token,channels,phase,created_at) VALUES(?,?,?,'deleting',?)",params![target,token,channels,now])?;
    tx.commit()?;
    Ok(token)
}

pub fn complete(path: &Path, target: &str, token: &str) -> Result<()> {
    let connection = crate::schema::open_initialized(path)?;
    if connection.execute("UPDATE cdr_archive_fences SET phase='deleted' WHERE target_thread_id=? AND token=? AND phase='deleting'",params![target,token])? != 1 {
        return Err(StoreError::Integrity("archive deletion fence ownership changed".into()));
    }
    Ok(())
}
