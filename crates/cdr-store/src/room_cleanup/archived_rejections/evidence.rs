use crate::Result;
use rusqlite::Connection;

pub(super) const CANDIDATES: &str = "SELECT ingress_id,payload_json,outcome_json,owner_user_id,canonical_owner,
 json_object('ingress_id',ingress_id,'version',version,'kind',kind,'event_id',event_id,
 'application_id',application_id,'channel_id',channel_id,'owner_user_id',owner_user_id,
 'source_message_id',source_message_id,'payload_json',payload_json,'runtime_id',runtime_id,
 'state',state,'phase',phase,'target_thread_id',target_thread_id,'canonical_owner',canonical_owner,
 'owner_kind',owner_kind,'owner_id',owner_id,'outcome_json',outcome_json,
 'confirmation_delivered',confirmation_delivered,'hold_reason',hold_reason,'notice_staged',notice_staged,
 'created_at',created_at,'updated_at',updated_at)
 FROM discord_ingress_journal j
 WHERE channel_id=?1 AND target_thread_id=?2 AND kind='interaction' AND state='held'
 AND phase='result_recorded' AND confirmation_delivered=0 AND owner_kind IS NULL AND owner_id IS NULL
 AND outcome_json IS NOT NULL AND canonical_owner IS NOT NULL AND event_id>0
 AND ingress_id='interaction:' || event_id
 AND NOT EXISTS(SELECT 1 FROM discord_ingress_owner_receipts r WHERE r.owner_key=j.canonical_owner)
 ORDER BY ingress_id";

pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS cdr_archived_cleanup_evidence (
         token TEXT NOT NULL, channel_id INTEGER NOT NULL, target_thread_id TEXT NOT NULL,
         ingress_id TEXT NOT NULL, payload_json TEXT NOT NULL, outcome_json TEXT NOT NULL,
         row_snapshot_json TEXT NOT NULL, archive_json TEXT NOT NULL, created_at REAL NOT NULL,
         PRIMARY KEY(token,ingress_id));
         CREATE INDEX IF NOT EXISTS cdr_archived_cleanup_evidence_ingress
         ON cdr_archived_cleanup_evidence(ingress_id);
         CREATE TRIGGER IF NOT EXISTS cdr_archived_cleanup_evidence_no_update
         BEFORE UPDATE ON cdr_archived_cleanup_evidence
         BEGIN SELECT RAISE(ABORT,'archived cleanup evidence is append-only'); END;
         CREATE TRIGGER IF NOT EXISTS cdr_archived_cleanup_evidence_no_delete
         BEFORE DELETE ON cdr_archived_cleanup_evidence
         BEGIN SELECT RAISE(ABORT,'archived cleanup evidence is append-only'); END;",
    )?;
    Ok(())
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='cdr_archived_cleanup_evidence')",
        [], |row| row.get(0),
    )?;
    let indexed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index' AND name='cdr_archived_cleanup_evidence_ingress')",
        [], |row| row.get(0),
    )?;
    Ok(exists
        && indexed
        && super::super::schema::has_trigger(
            connection,
            "cdr_archived_cleanup_evidence_no_update",
        )?
        && super::super::schema::has_trigger(
            connection,
            "cdr_archived_cleanup_evidence_no_delete",
        )?)
}
