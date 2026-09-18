//! Unbound observations are data, never permission to reply or show controls.
use super::{NewQuestion, QuestionBody, invalid, occurrence_id};
use crate::{Result, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
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
    // Count ALL active owners, including historical and quarantined ones. The
    // sole job is only a candidate: a new-generation event may precede handoff.
    // Capture original execution provenance without rewriting it to event time.
    let jobs = tx.prepare("SELECT job_id,channel_id,owner_user_id,app_server_generation,execution_generation,attempt_count FROM codex_turn_queue WHERE target_thread_id=? AND state!='pending'")?
        .query_map([n.thread_id], |r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,i64>(3)?,r.get::<_,Option<i64>>(4)?,r.get::<_,i64>(5)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let [(job, channel, Some(owner), original_generation, execution_generation, attempt)] =
        jobs.as_slice()
    else {
        return Err(invalid(
            "async question has no unique original Discord job candidate; no controls sent",
        ));
    };
    tx.execute("INSERT INTO cdr_async_question_inbox (id,runtime_id,generation,thread_id,turn_id,item_id,candidate_job_id,candidate_channel_id,candidate_owner_id,body,created_at,candidate_generation,candidate_execution_generation,candidate_attempt_count) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![id,n.runtime_id,n.generation,n.thread_id,n.turn_id,n.item_id,job,channel,owner,body,n.now,original_generation,execution_generation,attempt])?;
    tx.commit()?;
    Ok(())
}

/// Promote only exact pinned-job ownership, under the same `SQLite` transaction.
/// Neither a new job in the same thread nor a merely observed successor suffices.
pub fn reconcile_observations(path: &Path, runtime: &str, generation: i64) -> Result<usize> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let inserted = reconcile_in(&tx, Some(runtime), Some(generation), None)?;
    tx.commit()?;
    Ok(inserted)
}

/// Preserve ownership before the exact completed job is removed. A generationless
/// outbox row is never promoted into new question authority after deletion.
pub(crate) fn reconcile_job_in(db: &Connection, job_id: &str) -> Result<()> {
    reconcile_in(db, None, None, Some(job_id))?;
    Ok(())
}

fn reconcile_in(
    db: &Connection,
    runtime: Option<&str>,
    generation: Option<i64>,
    job: Option<&str>,
) -> Result<usize> {
    let inserted = db.execute("INSERT OR IGNORE INTO cdr_async_questions
        (id,runtime_id,generation,thread_id,turn_id,item_id,origin_job_id,channel_id,owner_user_id,body,owner_confirmed,created_at,updated_at)
        SELECT i.id,i.runtime_id,i.generation,i.thread_id,i.turn_id,i.item_id,i.candidate_job_id,i.candidate_channel_id,i.candidate_owner_id,i.body,1,i.created_at,i.created_at
        FROM cdr_async_question_inbox i WHERE (?1 IS NULL OR i.runtime_id=?1) AND (?2 IS NULL OR i.generation=?2)
        AND (?3 IS NULL OR i.candidate_job_id=?3) AND i.state='waiting'
        AND EXISTS(SELECT 1 FROM codex_turn_queue q WHERE q.job_id=i.candidate_job_id AND q.target_thread_id=i.thread_id
            AND q.turn_id=i.turn_id AND q.state='running' AND q.goal_waiting=0
            AND q.channel_id=i.candidate_channel_id AND q.owner_user_id=i.candidate_owner_id
            AND (q.turn_observation_generation=i.generation OR (q.turn_observation_generation IS NULL AND q.app_server_generation=i.generation))
            AND ((i.candidate_generation IS NULL AND i.candidate_attempt_count IS NULL AND q.app_server_generation=i.generation)
                OR (q.app_server_generation=i.candidate_generation AND q.execution_generation IS i.candidate_execution_generation AND q.attempt_count=i.candidate_attempt_count)))
        AND (SELECT COUNT(*) FROM codex_turn_queue q WHERE q.target_thread_id=i.thread_id AND q.state!='pending')=1",
        params![runtime,generation,job])?;
    db.execute("DELETE FROM cdr_async_question_inbox WHERE (?1 IS NULL OR runtime_id=?1) AND (?2 IS NULL OR generation=?2)
        AND (?3 IS NULL OR candidate_job_id=?3) AND state='waiting'
        AND EXISTS(SELECT 1 FROM cdr_async_questions q WHERE q.id=cdr_async_question_inbox.id
        AND q.runtime_id=cdr_async_question_inbox.runtime_id AND q.generation=cdr_async_question_inbox.generation
        AND q.origin_job_id=cdr_async_question_inbox.candidate_job_id AND q.channel_id=cdr_async_question_inbox.candidate_channel_id
        AND q.owner_user_id=cdr_async_question_inbox.candidate_owner_id AND q.body=cdr_async_question_inbox.body AND q.owner_confirmed=1)",
        params![runtime,generation,job])?;
    Ok(inserted)
}
