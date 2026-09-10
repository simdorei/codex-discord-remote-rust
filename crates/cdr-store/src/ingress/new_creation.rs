use crate::{Result, StoreError};
use rusqlite::{TransactionBehavior, params};
use std::path::Path;

/// Freeze the resolved context before thread/start. Only its winning attempt
/// may write it, once. Subsequent verification must never re-resolve a project.
pub fn record_new_creation(
    path: &Path,
    key: &str,
    generation: i64,
    cwd: Option<&str>,
    channel: i64,
    now: f64,
) -> Result<()> {
    if cwd.is_some_and(|value| value.trim().is_empty()) || channel <= 0 {
        return Err(StoreError::Integrity("invalid new creation context".into()));
    }
    let context = serde_json::json!({"version":1,"cwd":cwd,"origin_channel_id":channel});
    let mut connection = crate::schema::open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ingress = super::read::get_in(&transaction, key)?
        .ok_or_else(|| StoreError::Integrity("new creation ingress is missing".into()))?;
    let origin =
        serde_json::to_value(crate::mapping::new_thread_origin_in(&transaction, channel)?)?;
    if ingress.payload.get("new_origin") != Some(&origin) {
        return Err(StoreError::Integrity(
            "new origin mapping changed after classification/admission; no thread/start permitted"
                .into(),
        ));
    }
    let changed = transaction.execute(
        "UPDATE discord_ingress_journal
         SET outcome_json=json_set(outcome_json,'$.new_creation',json(?)),updated_at=?
         WHERE ingress_id=? AND state='executing' AND phase='thread/start'
         AND json_extract(outcome_json,'$.thread_start_generation')=?
         AND json_type(outcome_json,'$.new_creation') IS NULL AND channel_id=?",
        params![context.to_string(), now, key, generation, channel],
    )?;
    if changed != 1 {
        return Err(StoreError::Integrity(
            "new creation context has no matching unwritten attempt".into(),
        ));
    }
    transaction.commit()?;
    Ok(())
}
