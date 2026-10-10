//! An unbound legacy Stop cannot acquire authority over a later, completed input.
//! Keep the old ingress and per-request holds; neither time nor restart clears them.
use crate::Result;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub(super) fn superseded_in(
    db: &Connection,
    thread: &str,
    ingress: &str,
    payload: &Value,
) -> Result<bool> {
    if payload.get("version").and_then(Value::as_u64) != Some(1)
        || payload.get("plan") != Some(&json!({"Execute":{"Stop":{"reference":null}}}))
        || ["lifecycle_binding", "stop_origin", "work", "command"]
            .iter()
            .any(|key| payload.get(*key).is_some_and(|value| !value.is_null()))
        || !super::has_table(db, "cdr_recovery_ingress_order")?
    {
        return Ok(false);
    }
    super::super::admission_order::check_compatibility_in(
        db,
        super::super::admission_order::FORMAT_VERSION,
    )?;
    // Legacy ordinals precede genuinely new admissions; legacy-to-legacy order
    // is not sufficient. Bind the later input to the certified terminal's sealed
    // original job, not the question's timestamp or a displayed completion label.
    Ok(db.query_row(
        "SELECT EXISTS(
         SELECT 1 FROM discord_ingress_journal old
         JOIN cdr_recovery_ingress_order prior ON prior.ingress_id=old.ingress_id
             AND prior.kind=old.kind AND prior.event_id=old.event_id AND prior.origin='legacy'
         JOIN discord_ingress_journal input ON input.target_thread_id=old.target_thread_id
             AND input.channel_id=old.channel_id AND input.owner_user_id=old.owner_user_id
             AND input.kind='message' AND input.event_id=input.source_message_id
             AND input.state='owned' AND input.owner_kind='prompt'
         JOIN cdr_recovery_ingress_order newer ON newer.ingress_id=input.ingress_id
             AND newer.kind=input.kind AND newer.event_id=input.event_id
             AND newer.origin='admitted' AND newer.sequence>prior.sequence
         JOIN cdr_async_execution_obligations o ON o.origin_job_id=input.owner_id
             AND o.thread_id=input.target_thread_id AND o.channel_id=input.channel_id
         JOIN cdr_async_terminal_settlements s ON s.question_id=o.question_id
             AND s.revision=o.revision AND s.proof_json=o.terminal_proof_json
         WHERE old.ingress_id=?1 AND old.target_thread_id=?2
             AND old.kind='message' AND old.event_id=old.source_message_id
             AND o.format_version=1 AND o.policy='ordinary'
             AND o.execution_state='terminal' AND o.admission_state='settled'
             AND json_extract(o.original_seal,'$.identity.job.job_id')=o.origin_job_id
             AND json_extract(o.original_seal,'$.identity.job.target_thread_id')=o.thread_id
             AND json_extract(o.original_seal,'$.identity.job.turn_id')=o.turn_id
             AND json_extract(o.original_seal,'$.identity.job.channel_id')=input.channel_id
             AND json_extract(o.original_seal,'$.identity.job.owner_user_id')=input.owner_user_id
             AND json_extract(o.original_seal,'$.identity.job.discord_message_id')=input.event_id
             AND json_extract(o.original_seal,'$.identity.job.state')='Running'
             AND json_extract(o.original_seal,'$.identity.job.attempt_count')>0)",
        params![ingress, thread],
        |row| row.get(0),
    )?)
}
