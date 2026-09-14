//! Unbound observations are data, never permission to reply or show controls.
use super::{NewQuestion, QuestionBody, invalid, occurrence_id};
use crate::{Result, schema::open_initialized};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::path::Path;

pub fn record_observation(path: &Path, n: &NewQuestion<'_>) -> Result<()> {
    let body = super::observe::encode(n)?;
    let id = occurrence_id(n.thread_id, n.turn_id, n.item_id, n.body.index)?;
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<String> = tx.query_row(
        "SELECT body FROM cdr_async_questions WHERE id=? UNION ALL SELECT body FROM cdr_async_question_inbox WHERE id=? LIMIT 1",
        params![id,id], |r|r.get(0)).optional()?;
    if let Some(existing) = existing {
        if serde_json::from_str::<QuestionBody>(&existing)? != *n.body {
            return Err(invalid("async question occurrence changed its content"));
        }
        return Ok(()); // Never change a pinned candidate or revive an old owner.
    }
    // Pin one existing bot job as a CANDIDATE only. This permits journalling a
    // successor before the goal handoff, but grants no actor/channel authority.
    // Reconciliation below requires proof for this exact job AND question turn.
    let jobs = tx.prepare("SELECT job_id,channel_id,owner_user_id FROM codex_turn_queue WHERE target_thread_id=? AND app_server_generation=? AND owner_user_id IS NOT NULL AND state IN ('starting','running')")?
        .query_map(params![n.thread_id,n.generation], |r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let [(job, channel, owner)] = jobs.as_slice() else {
        return Err(invalid(
            "async question has no unique original Discord job candidate; no controls sent",
        ));
    };
    tx.execute("INSERT INTO cdr_async_question_inbox (id,runtime_id,generation,thread_id,turn_id,item_id,candidate_job_id,candidate_channel_id,candidate_owner_id,body,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        params![id,n.runtime_id,n.generation,n.thread_id,n.turn_id,n.item_id,job,channel,owner,body,n.now])?;
    tx.commit()?;
    Ok(())
}

/// Promote only exact pinned-job ownership, under the same `SQLite` transaction.
/// Neither a new job in the same thread nor a merely observed successor suffices.
pub fn reconcile_observations(path: &Path, runtime: &str, generation: i64) -> Result<usize> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let inserted = tx.execute("INSERT OR IGNORE INTO cdr_async_questions
        (id,runtime_id,generation,thread_id,turn_id,item_id,origin_job_id,channel_id,owner_user_id,body,owner_confirmed,created_at,updated_at)
        SELECT i.id,i.runtime_id,i.generation,i.thread_id,i.turn_id,i.item_id,i.candidate_job_id,i.candidate_channel_id,i.candidate_owner_id,i.body,1,i.created_at,i.created_at
        FROM cdr_async_question_inbox i WHERE i.runtime_id=?1 AND i.generation=?2 AND i.state='waiting'
        AND (EXISTS(SELECT 1 FROM codex_turn_queue q WHERE q.job_id=i.candidate_job_id AND q.target_thread_id=i.thread_id AND q.turn_id=i.turn_id AND q.app_server_generation=i.generation AND q.state='running' AND q.goal_waiting=0 AND q.channel_id=i.candidate_channel_id AND q.owner_user_id=i.candidate_owner_id)
        OR EXISTS(SELECT 1 FROM codex_delivery_outbox d WHERE d.job_id=i.candidate_job_id AND d.target_thread_id=i.thread_id AND d.turn_id=i.turn_id AND d.channel_id=i.candidate_channel_id))",
        params![runtime,generation])?;
    tx.execute("DELETE FROM cdr_async_question_inbox WHERE runtime_id=?1 AND generation=?2 AND state='waiting' AND EXISTS(SELECT 1 FROM cdr_async_questions q WHERE q.id=cdr_async_question_inbox.id AND q.runtime_id=cdr_async_question_inbox.runtime_id AND q.generation=cdr_async_question_inbox.generation AND q.origin_job_id=cdr_async_question_inbox.candidate_job_id AND q.body=cdr_async_question_inbox.body AND q.owner_confirmed=1)",params![runtime,generation])?;
    tx.commit()?;
    Ok(inserted)
}
