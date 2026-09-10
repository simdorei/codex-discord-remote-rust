use super::{new_command_prompt, read::get_in};
use crate::{Result, StoreError, queue::NewQueueJob};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

/// Store the exact prepared first-input fingerprint in the same transaction
/// that promotes its intake. No second copy of the prompt is retained here.
pub(crate) fn record_new_evidence(connection: &Connection, job: &NewQueueJob<'_>) -> Result<()> {
    let keys=connection.prepare("SELECT ingress_id FROM discord_ingress_journal WHERE owner_kind='prompt' AND owner_id=?")?
        .query_map([job.job_id],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for key in keys {
        let saved = get_in(connection, &key)?
            .ok_or_else(|| StoreError::Integrity("missing new evidence owner".into()))?;
        if new_command_prompt(&saved).is_none() {
            continue;
        }
        if saved.target_thread_id.as_deref() != Some(job.target_thread_id)
            || saved.owner_user_id != job.owner_user_id.unwrap_or_default()
        {
            return Err(StoreError::Integrity(
                "new evidence owner identity changed".into(),
            ));
        }
        let digest = hex::encode(Sha256::digest(job.prompt.as_bytes()));
        let evidence = serde_json::json!({"prompt_sha256":digest,"thread_id":job.target_thread_id,"channel_id":job.channel_id});
        crate::new_reply::promote_in(connection, &saved, job, &digest)?;
        if let Some(old) = saved
            .outcome
            .as_ref()
            .and_then(|v| v.get("new_verification"))
            && old != &evidence
        {
            return Err(StoreError::Integrity(
                "new prepared input evidence changed".into(),
            ));
        }
        connection.execute("UPDATE discord_ingress_journal SET outcome_json=json_set(COALESCE(outcome_json,'{}'),'$.new_verification',json(?)) WHERE ingress_id=?",params![evidence.to_string(),key])?;
    }
    Ok(())
}
