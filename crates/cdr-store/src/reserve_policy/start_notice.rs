//! A rejected start has no turn. Its failure receipt must not invent turn ownership.
use crate::{Result, StoreError, queue::StoredQueueJob, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const DOMAIN: &str = "reserve/start-failure/v1";

#[derive(Clone, Debug)]
pub struct StartNotice {
    pub job_id: String,
    pub thread_id: String,
    pub channel_id: i64,
    pub content: String,
}

pub(super) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS codex_reserve_start_notices (
        job_id TEXT PRIMARY KEY, target_thread_id TEXT NOT NULL, channel_id INTEGER NOT NULL,
        app_server_generation INTEGER NOT NULL, attempt_count INTEGER NOT NULL,
        content TEXT NOT NULL, created_at REAL NOT NULL DEFAULT(unixepoch())
    );",
    )?;
    Ok(())
}

pub(super) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row("SELECT COUNT(*)=7 FROM pragma_table_info('codex_reserve_start_notices')
        WHERE name IN ('job_id','target_thread_id','channel_id','app_server_generation','attempt_count','content','created_at')",[],|r|r.get(0))?)
}

/// Called in the same transaction as the CAS that records the usage failure.
pub(crate) fn stage_in(connection: &Connection, job: &StoredQueueJob, reason: &str) -> Result<()> {
    let reason: String = reason
        .strip_prefix(crate::execution_hold::PREFIX)
        .or_else(|| reason.strip_prefix(super::HOLD_PREFIX))
        .unwrap_or(reason)
        .chars()
        .take(700)
        .collect();
    let content = format!(
        "Failed\n사용량 한도로 요청 시작이 거절됐습니다.\njob: {}\n{reason}\n이 요청은 자동 재실행하지 않습니다. 필요하면 모델을 수동으로 변경한 뒤 새 요청을 보내세요.",
        job.job_id
    );
    connection.execute(
        "INSERT OR IGNORE INTO codex_reserve_start_notices
        (job_id,target_thread_id,channel_id,app_server_generation,attempt_count,content)
        VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            job.job_id,
            job.target_thread_id,
            job.channel_id,
            job.app_server_generation,
            job.attempt_count,
            content
        ],
    )?;
    Ok(())
}

pub fn pending(path: &Path) -> Result<Vec<StartNotice>> {
    let connection = open_initialized(path)?;
    let mut statement = connection.prepare(
        "SELECT job_id,target_thread_id,channel_id,content
        FROM codex_reserve_start_notices ORDER BY created_at,job_id",
    )?;
    Ok(statement
        .query_map([], |r| {
            Ok(StartNotice {
                job_id: r.get(0)?,
                thread_id: r.get(1)?,
                channel_id: r.get(2)?,
                content: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn complete(path: &Path, job_id: &str) -> Result<()> {
    open_initialized(path)?.execute(
        "DELETE FROM codex_reserve_start_notices WHERE job_id=?1",
        [job_id],
    )?;
    Ok(())
}

/// Receipt claim and original no-turn custody are checked under one DB writer lock.
/// Returns the job identity so the caller can additionally validate /new ingress.
pub(crate) fn validate_claim_in(
    connection: &Connection,
    key: &str,
    hash: &str,
) -> Result<Option<String>> {
    let Ok((channel, domain, job_id, chunk)) =
        serde_json::from_str::<(i64, String, String, usize)>(key)
    else {
        return Ok(None);
    };
    if domain != DOMAIN {
        return Ok(None);
    }
    // Generation adoption changes only the queue's current connection. This is
    // delivery of a recorded historical rejection, not permission to start or
    // switch models. Immutable job/target/channel/attempt and held no-turn state
    // remain mandatory; the notice retains its original generation as evidence.
    let row: Option<(String, i64, String)> = connection
        .query_row(
            "SELECT n.target_thread_id,n.channel_id,n.content FROM codex_reserve_start_notices n
         JOIN codex_turn_queue q ON q.job_id=n.job_id AND q.target_thread_id=n.target_thread_id
         AND q.channel_id=n.channel_id
         AND q.attempt_count=n.attempt_count WHERE n.job_id=?1 AND q.state='pending'
         AND q.turn_id IS NULL AND (EXISTS(SELECT 1 FROM cdr_execution_holds h WHERE h.job_id=q.job_id) OR substr(q.last_error,1,length(?2))=?2)",
            params![job_id, super::HOLD_PREFIX],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((thread, original_channel, content)) = row else {
        return Err(StoreError::Integrity(
            "Reserve failure notice has no exact held no-turn job".into(),
        ));
    };
    if chunk != 0
        || original_channel != channel
        || hex::encode(Sha256::digest(content.as_bytes())) != hash
    {
        return Err(StoreError::Integrity(
            "Reserve failure notice content or destination changed".into(),
        ));
    }
    crate::dead_generation::ensure_target_available(connection, &thread)?;
    let mapping = crate::mapping::mirrored_thread_id_in(connection, Some(channel))?;
    if mapping.is_some_and(|actual| actual != thread) {
        return Err(StoreError::Integrity(
            "Reserve failure notice room was remapped".into(),
        ));
    }
    let cancelled: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM discord_ingress_journal
        WHERE owner_kind='prompt' AND owner_id=?1 AND phase='cancelled')",
        [&job_id],
        |r| r.get(0),
    )?;
    if cancelled {
        return Err(StoreError::Integrity(
            "Reserve failure notice belongs to cancelled ingress".into(),
        ));
    }
    Ok(Some(job_id))
}
