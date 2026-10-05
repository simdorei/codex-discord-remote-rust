use std::path::Path;

use rusqlite::params;

use super::{Entry, MAX_PAYLOAD_BYTES, Source, current};
use crate::{Result, StoreError, schema::open_initialized};

pub enum Payload {
    Observed { generation: i64, json: String },
    Commentary(crate::commentary_outbox::PendingCommentary),
    Goal(crate::goal_progress::PendingProgress),
    StartFailure(crate::reserve_policy::start_notice::StartNotice),
    Question(Box<crate::async_question::Question>),
    Final(crate::delivery::StoredDelivery),
}

/// Revalidate the source head and payload bound in one `SQLite` read snapshot.
pub fn load(path: &Path, entry: &Entry, runtime: &str, generation: i64) -> Result<Option<Payload>> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction()?;
    let Some(actual) = current(&tx, entry, runtime, generation)? else {
        return Ok(None);
    };
    if actual != *entry {
        return Ok(None);
    }
    if actual.bytes > MAX_PAYLOAD_BYTES {
        return Err(StoreError::Integrity(
            "completion payload exceeds in-memory budget; durable evidence retained".into(),
        ));
    }
    let payload = match entry.source {
        Source::Queue | Source::AsyncOrphan => return Ok(None),
        Source::Observed => tx.query_row(
            "SELECT generation,payload FROM codex_observed_completions WHERE thread_id=? AND turn_id=?",
            params![entry.target,entry.turn],
            |r| Ok(Payload::Observed { generation:r.get(0)?,json:r.get(1)? }),
        )?,
        Source::Commentary => tx.query_row(
            "SELECT sequence,job_id,target_thread_id,turn_id,channel_id,text
             FROM codex_commentary_outbox WHERE sequence=?",
            [&entry.id], |r| Ok(Payload::Commentary(crate::commentary_outbox::PendingCommentary {
                sequence:r.get(0)?,job_id:r.get(1)?,thread_id:r.get(2)?,
                turn_id:r.get(3)?,channel_id:r.get(4)?,text:r.get(5)?,
            })),
        )?,
        Source::Goal => tx.query_row(
            "SELECT thread,turn,channel,content,last_error,job_id FROM codex_goal_progress
             WHERE thread=? AND turn=?",
            params![entry.target,entry.turn],
            |r| Ok(Payload::Goal(crate::goal_progress::PendingProgress {
                thread:r.get(0)?,turn:r.get(1)?,channel:r.get(2)?,
                content:r.get(3)?,last_error:r.get(4)?,job_id:r.get(5)?,
            })),
        )?,
        Source::StartFailure => tx.query_row(
            "SELECT job_id,target_thread_id,channel_id,content
             FROM codex_reserve_start_notices WHERE job_id=?",
            [&entry.id], |r| Ok(Payload::StartFailure(
                crate::reserve_policy::start_notice::StartNotice {
                    job_id:r.get(0)?,thread_id:r.get(1)?,channel_id:r.get(2)?,content:r.get(3)?,
                },
            )),
        )?,
        Source::Question => Payload::Question(Box::new(crate::async_question::read(&tx,&entry.id)?)),
        Source::Final => Payload::Final(crate::delivery::select(&tx,&entry.id)?),
    };
    Ok(Some(payload))
}
