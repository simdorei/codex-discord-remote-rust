//! Exact local evidence snapshot, not proof of terminal or publisher exclusion.
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Value, json};

use super::invalid;
use crate::{Result, queue::QueueJobState};

const MAX_BYTES: usize = 262_144;
const MAX_ROWS: usize = 128;

pub(super) struct Captured {
    pub thread: String,
    pub owner: i64,
    pub channel: i64,
    pub seal: Value,
}

pub(super) fn capture_in(db: &Connection, job_id: &str) -> Result<Captured> {
    if db.is_autocommit() {
        return Err(crate::StoreError::ActiveTransaction);
    }
    let length: i64 = db.query_row(
        "SELECT length(CAST(prompt AS BLOB)) FROM codex_turn_queue WHERE job_id=?",
        [job_id],
        |r| r.get(0),
    )?;
    if length > 131_072 {
        return Err(invalid("pending input exceeds review bound"));
    }
    let job = crate::queue::select_job(db, job_id)?;
    let owner = job
        .owner_user_id
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("pending has no exact original owner"))?;
    if job.state != QueueJobState::Pending
        || job.channel_id <= 0
        || job.app_server_generation < 1
        || job.target_thread_id.trim().is_empty()
    {
        return Err(invalid("proposal does not name an owned Pending job"));
    }
    let mapped: bool = db.query_row(
        "SELECT count(*)=1 AND MAX(codex_thread_id=?1 AND discord_thread_id=?2)
         FROM mirror_threads WHERE codex_thread_id=?1 OR discord_thread_id=?2",
        rusqlite::params![job.target_thread_id, job.channel_id],
        |r| r.get(0),
    )?;
    if !mapped {
        return Err(invalid("pending mapping is missing or ambiguous"));
    }
    let target = &job.target_thread_id;
    let stop = crate::ingress::stop::revision::capture_in(db, Some(target))?;
    let mut budget = 0;
    let mut seal = serde_json::Map::new();
    for (key,sql) in [
        ("queue","SELECT * FROM codex_turn_queue WHERE target_thread_id=? ORDER BY created_at,job_id LIMIT 129"),
        ("mapping","SELECT * FROM mirror_threads WHERE codex_thread_id=? LIMIT 2"),
        ("obligations","SELECT * FROM cdr_async_execution_obligations WHERE thread_id=? ORDER BY question_id LIMIT 129"),
        ("settlements","SELECT * FROM cdr_async_terminal_settlements WHERE question_id IN
            (SELECT question_id FROM cdr_async_execution_obligations WHERE thread_id=?) ORDER BY question_id LIMIT 129"),
        ("handoffs","SELECT * FROM cdr_async_execution_handoffs WHERE question_id IN
            (SELECT question_id FROM cdr_async_execution_obligations WHERE thread_id=?) ORDER BY question_id,revision LIMIT 129"),
        ("policy","SELECT * FROM cdr_async_recovery_policies WHERE thread_id=? LIMIT 2"),
    ] {
        seal.insert(key.into(), rows_in(db, sql, target, &mut budget)?);
    }
    seal.insert("stop_origin".into(), stop);
    let seal = Value::Object(seal);
    if serde_json::to_vec(&seal)?.len() > MAX_BYTES {
        return Err(invalid("local evidence exceeds review bound"));
    }
    Ok(Captured {
        thread: job.target_thread_id,
        owner,
        channel: job.channel_id,
        seal,
    })
}

fn rows_in(db: &Connection, sql: &str, target: &str, budget: &mut usize) -> Result<Value> {
    let mut statement = db.prepare(sql)?;
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|v| (*v).into())
        .collect();
    let mut rows = statement.query([target])?;
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        if values.len() >= MAX_ROWS {
            return Err(invalid("local evidence page exceeds bound"));
        }
        let mut cells = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            let cell = match row.get_ref(index)? {
                ValueRef::Null => json!(["null"]),
                ValueRef::Integer(value) => json!(["integer", value]),
                ValueRef::Real(value) => json!(["real_bits", value.to_bits().to_string()]),
                ValueRef::Text(value) => {
                    if value.len() > MAX_BYTES {
                        return Err(invalid("evidence cell exceeds bound"));
                    }
                    let text =
                        std::str::from_utf8(value).map_err(|_| invalid("non-UTF8 evidence"))?;
                    json!(["text", text])
                }
                ValueRef::Blob(value) => {
                    if value.len() > MAX_BYTES / 2 {
                        return Err(invalid("evidence blob exceeds bound"));
                    }
                    json!(["blob_hex", hex::encode(value)])
                }
            };
            *budget = budget.saturating_add(serde_json::to_vec(&cell)?.len());
            if *budget > MAX_BYTES {
                return Err(invalid("local evidence exceeds review bound"));
            }
            cells.push(cell);
        }
        values.push(cells);
    }
    Ok(json!({"columns":columns,"rows":values}))
}
