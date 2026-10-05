//! One read-only snapshot of controls and holds, never recovery authority.
mod intent;
use super::{BYTE_LIMIT, InspectResult, hash, page, table};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

const METADATA_LIMIT: i64 = 4096;

pub(super) fn inspect(db: &Connection, thread: &str) -> InspectResult<Value> {
    Ok(json!({
        "diagnostic_only":true,
        "state_labels_are_not_authority":true,
        "absence_is_clearance":false,
        "automatic_resume_authorized":false,
        "does_not_confirm_execution_end":true,
        "reviewed_incident_policy_required":
            thread==cdr_store::async_resolution::REVIEWED_INCIDENT_THREAD,
        "mapping":snapshot(db,thread,"mirror_threads",
            "SELECT json_object('thread_id',codex_thread_id,
             'channel_id',discord_channel_id,'discord_thread_id',discord_thread_id,
             'same_discord_thread_mappings',(SELECT COUNT(*) FROM mirror_threads other
                 WHERE other.discord_thread_id=mirror_threads.discord_thread_id)) AS metadata,
             NULL AS details FROM mirror_threads WHERE codex_thread_id=?1 ORDER BY codex_thread_id")?,
        "recovery_policy":snapshot(db,thread,"cdr_async_recovery_policies",
            "SELECT json_object('thread_id',thread_id,'format_version',format_version,
             'policy',policy,'proposal_sha256',proposal_sha256,'original_turn_id',original_turn_id,
             'origin_job_id',origin_job_id,'pending_job_id',pending_job_id) AS metadata,
             NULL AS details FROM cdr_async_recovery_policies WHERE thread_id=?1 ORDER BY thread_id")?,
        "stop_receipts":snapshot(db,thread,"cdr_stop_revision_receipts",
            "SELECT json_object('operation_id',operation_id,'thread_id',target_thread_id,
             'revision',revision) AS metadata,scope_json AS details
             FROM cdr_stop_revision_receipts WHERE target_thread_id=?1 ORDER BY revision,operation_id")?,
        "stop_controls":snapshot(db,thread,"cdr_stop_controls",
            "SELECT json_object('sequence',sequence,'operation_id',operation_id,
             'thread_id',target_thread_id,'resident',resident_owner,'generation',generation,
             'turn_id',turn_id,'phase',phase) AS metadata,record_json AS details
             FROM cdr_stop_controls WHERE target_thread_id=?1 ORDER BY sequence")?,
        "archive_fences":snapshot(db,thread,"codex_archive_fences",
            "SELECT json_object('thread_id',target_thread_id,'operation_id',operation_id,
             'own_ingress_id',own_ingress_id,'phase',phase) AS metadata,NULL AS details
             FROM codex_archive_fences WHERE target_thread_id=?1 ORDER BY target_thread_id")?,
        "execution_holds":snapshot(db,thread,"cdr_execution_holds",
            "SELECT json_object('job_id',job_id,'thread_id',target_thread_id) AS metadata,
             evidence_json AS details FROM cdr_execution_holds
             WHERE target_thread_id=?1 ORDER BY job_id")?,
        "unresolved_ingress":snapshot(db,thread,"discord_ingress_journal",
            "SELECT json_object('ingress_id',ingress_id,'thread_id',target_thread_id,
             'kind',kind,'event_id',event_id,'source_message_id',source_message_id,
             'target_is_unbound',target_thread_id IS NULL,'state',state,'phase',phase,
             'channel_id',channel_id,'owner_user_id',owner_user_id,
             'owner_kind',owner_kind,'owner_id',owner_id,
             'confirmation_delivered',confirmation_delivered,
             'declared_action',CASE WHEN json_valid(payload_json) THEN
                 CASE WHEN json_type(payload_json,'$.plan.Execute.Stop')='object' THEN 'Stop'
                      WHEN json_type(payload_json,'$.plan.Execute.Archive')='object' THEN 'Archive'
                      ELSE 'unknown_or_other' END
                 ELSE 'unknown_or_other' END) AS metadata,payload_json AS details
             FROM discord_ingress_journal
             WHERE (target_thread_id=?1 OR target_thread_id IS NULL)
             AND state!='completed' AND NOT(state='owned' AND confirmation_delivered=1)
             ORDER BY created_at,ingress_id")?
    }))
}

fn snapshot(db: &Connection, thread: &str, name: &str, query: &str) -> InspectResult<Value> {
    if !table(db, name)? {
        let mut value = page(Vec::new());
        value["schema_present"] = json!(false);
        return Ok(value);
    }
    // Static internal SQL only. Result metadata and opaque details are bounded
    // separately; prompts/selections are hashed, never copied into the report.
    let sql = format!(
        "SELECT CASE WHEN length(CAST(metadata AS BLOB))<=?3 THEN metadata END,
        length(CAST(details AS BLOB)),
        CASE WHEN length(CAST(details AS BLOB))<=?2 THEN details END
        FROM ({query}) LIMIT 129"
    );
    let mut statement = db.prepare(&sql)?;
    let mut cursor = statement.query(params![thread, BYTE_LIMIT, METADATA_LIMIT])?;
    let mut records = Vec::new();
    while let Some(row) = cursor.next()? {
        let metadata: Option<String> = row.get(0)?;
        let bytes: Option<i64> = row.get(1)?;
        let details: Option<String> = row.get(2)?;
        let mut value = match metadata {
            Some(value) => serde_json::from_str::<Value>(&value)?,
            None => json!({"metadata_oversized":true}),
        };
        value["details_bytes"] = json!(bytes);
        value["details_oversized"] = json!(bytes.is_some_and(|size| size > BYTE_LIMIT));
        value["details_sha256"] = json!(hash(details.as_deref()));
        if name == "discord_ingress_journal" {
            value["intent_evidence"] = intent::inspect(&value, details.as_deref());
        }
        records.push(value);
    }
    let mut value = page(records);
    value["schema_present"] = json!(true);
    value["metadata_byte_limit"] = json!(METADATA_LIMIT);
    value["details_byte_limit"] = json!(BYTE_LIMIT);
    Ok(value)
}
