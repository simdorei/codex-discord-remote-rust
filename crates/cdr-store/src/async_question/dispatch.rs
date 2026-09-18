use super::{Question, invalid, read, validate_mapping};
use crate::Result;
use crate::queue::{NewQueueJob, QueueJobState};
use crate::schema::open_initialized;
use rusqlite::{TransactionBehavior, params};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchMode {
    Steer,
    Start,
}

pub struct Claim<'a> {
    pub id: &'a str,
    pub runtime_id: &'a str,
    pub generation: i64,
    pub channel: i64,
    pub actor: i64,
    pub message: &'a str,
    pub option: usize,
    pub mode: DispatchMode,
    pub prompt: &'a str,
    pub now: f64,
}

pub fn begin_dispatch(path: &Path, c: &Claim<'_>) -> Result<Question> {
    begin_dispatch_inner(path, c, None)
}

pub fn begin_dispatch_prepared(
    path: &Path,
    c: &Claim<'_>,
    expected: &crate::reserve_policy::admission::Stamp,
) -> Result<Question> {
    begin_dispatch_inner(path, c, Some(expected))
}

fn begin_dispatch_inner(
    path: &Path,
    c: &Claim<'_>,
    expected: Option<&crate::reserve_policy::admission::Stamp>,
) -> Result<Question> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let q = read(&tx, c.id)?;
    if q.runtime_id != c.runtime_id
        || q.generation != c.generation
        || q.channel_id != c.channel
        || q.owner_user_id != c.actor
        || q.message_id.as_deref() != Some(c.message)
        || c.option >= q.body.options.len()
        || !c.now.is_finite()
    {
        return Err(invalid(
            "question actor, room, message, connection or option does not match",
        ));
    }
    if q.state != "open" {
        return Err(invalid(&format!(
            "question answer is {}; no new answer sent. {}",
            q.state, q.error
        )));
    }
    validate_mapping(&tx, &q)?;
    let preparation = crate::reserve_policy::admission::capture_in(&tx, &q.thread_id)?;
    if expected.is_some_and(|expected| expected != &preparation) {
        return Err(invalid("async reply preparation changed before claim"));
    }
    let jobs = crate::queue::read::all_jobs(&tx)?
        .into_iter()
        .filter(|job| job.target_thread_id == q.thread_id)
        .collect::<Vec<_>>();
    let reply_job = if c.mode == DispatchMode::Start {
        if !jobs.is_empty() {
            return Err(invalid(
                "original final delivery or later work is still pending; no new turn started",
            ));
        }
        let job_id = format!("async-question:{}", q.id);
        crate::queue::enqueue_in_transaction(
            &tx,
            NewQueueJob {
                job_id: &job_id,
                target_thread_id: &q.thread_id,
                channel_id: q.channel_id,
                owner_user_id: Some(q.owner_user_id),
                discord_message_id: None,
                app_server_generation: q.generation,
                prompt: c.prompt,
                queued: false,
                ack_sent: true,
                created_at: c.now,
            },
        )?;
        // Reuse the existing backward-compatible quarantine encoding. This job
        // must never enter generic Starting recovery or automatic retry/adoption.
        tx.execute(
            "UPDATE codex_turn_queue SET state='running',turn_id=?,last_error=? WHERE job_id=?",
            params![
                format!("{}async:{}", crate::queue::QUARANTINED_TURN_PREFIX, q.id),
                format!(
                    "{}async question answer dispatch unconfirmed; no automatic retry",
                    crate::queue::QUARANTINED_ERROR_PREFIX
                ),
                job_id
            ],
        )?;
        Some(job_id)
    } else {
        let active: Vec<_> = jobs
            .iter()
            .filter(|j| j.state != QueueJobState::Pending)
            .collect();
        if active.len() != 1 || !super::ownership::running_matches(active[0], &q) {
            return Err(invalid(
                "the question's exact original running job is no longer owned",
            ));
        }
        None
    };
    let option = i64::try_from(c.option).map_err(|_| invalid("invalid option index"))?;
    tx.execute("UPDATE cdr_async_questions SET state='dispatching',chosen=?,dispatch_mode=?,reply_job_id=?,updated_at=? WHERE id=? AND state='open'",
        params![option,if c.mode==DispatchMode::Start {"start"} else {"steer"},reply_job,c.now,c.id])?;
    super::guard::seal_in(&tx, c.id, preparation)?;
    let claimed = read(&tx, c.id)?;
    tx.commit()?;
    Ok(claimed)
}

pub fn confirm_dispatch(path: &Path, id: &str, turn: &str) -> Result<()> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let q = read(&tx, id)?;
    if q.state != "dispatching" || turn.trim().is_empty() {
        return Err(invalid("answer dispatch is not awaiting confirmation"));
    }
    super::guard::verify_identity_in(&tx, &q)?;
    if let Some(job) = &q.reply_job_id {
        if turn == q.turn_id {
            return Err(invalid(
                "new question reply returned the original turn identity",
            ));
        }
        if tx.execute("UPDATE codex_turn_queue SET state='running',turn_id=?,last_error='',updated_at=unixepoch() WHERE job_id=? AND turn_id=? AND app_server_generation=?",
            params![turn,job,format!("{}async:{}",crate::queue::QUARANTINED_TURN_PREFIX,q.id),q.generation])?!=1 {
            return Err(invalid("accepted question reply job changed; outcome held for review"));
        }
    } else if turn != q.turn_id {
        return Err(invalid("steer accepted a different turn"));
    }
    tx.execute("UPDATE cdr_async_questions SET state='submitted',accepted_turn_id=?,error='',updated_at=unixepoch() WHERE id=?",params![turn,id])?;
    tx.commit()?;
    Ok(())
}

pub fn record_error(path: &Path, id: &str, error: &str) -> Result<()> {
    open_initialized(path)?.execute(
        "UPDATE cdr_async_questions SET error=?,updated_at=unixepoch() WHERE id=?",
        params![error.chars().take(1000).collect::<String>(), id],
    )?;
    Ok(())
}

/// Only for pre-send failure or an authoritative RPC rejection, never a timeout.
pub fn reject_definite(path: &Path, id: &str, error: &str) -> Result<()> {
    reject_inner(path, id, error, false)
}

/// Fence and rejection are atomic: fence failure preserves the pending answer.
pub fn reject_usage_limit(path: &Path, id: &str, error: &str) -> Result<()> {
    reject_inner(path, id, error, true)
}

fn reject_inner(path: &Path, id: &str, error: &str, usage_limit: bool) -> Result<()> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let q = read(&tx, id)?;
    if q.state != "dispatching" {
        return Err(invalid("question dispatch state changed"));
    }
    super::guard::verify_identity_in(&tx, &q)?;
    if usage_limit {
        crate::reserve_policy::stage_usage_failure_in(&tx, &q.thread_id, error)?;
    }
    if let Some(job) = &q.reply_job_id {
        tx.execute(
            "DELETE FROM codex_turn_queue WHERE job_id=? AND turn_id=?",
            params![
                job,
                format!("{}async:{}", crate::queue::QUARANTINED_TURN_PREFIX, q.id)
            ],
        )?;
    }
    tx.execute(
        "UPDATE cdr_async_questions SET state='rejected',error=?,updated_at=unixepoch() WHERE id=?",
        params![error.chars().take(1000).collect::<String>(), id],
    )?;
    tx.commit()?;
    Ok(())
}
