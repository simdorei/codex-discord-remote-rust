//! Exact private evidence for one locally held Pending request.
use super::{api::database_identity, identity, invalid};
use crate::{Result, queue::QueueJobState};
use rusqlite::{Connection, params, types::ValueRef};
use serde_json::{Value, json};
use std::path::Path;

const MAX_BYTES: usize = 393_216;
const MAX_ROWS: usize = 128;

pub(super) struct Target {
    pub job: String,
    pub thread: String,
    pub owner: i64,
    pub channel: i64,
}

pub(super) struct Captured {
    pub target: Target,
    pub evidence: Value,
}

pub(super) fn capture_in(
    db: &Connection,
    path: &Path,
    job_id: &str,
    source: &str,
    creating: bool,
) -> Result<Captured> {
    if db.is_autocommit() {
        return Err(crate::StoreError::ActiveTransaction);
    }
    let length: i64 = db.query_row(
        "SELECT length(CAST(prompt AS BLOB)) FROM codex_turn_queue WHERE job_id=?",
        [job_id],
        |r| r.get(0),
    )?;
    if length > 131_072 {
        return Err(invalid("original input exceeds private evidence bound"));
    }
    let job = crate::queue::select_job(db, job_id)?;
    let owner = job
        .owner_user_id
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("original request owner is missing"))?;
    if job.state != QueueJobState::Pending
        || job.turn_id.is_some()
        || job.goal_waiting
        || job.discord_message_id.is_none_or(|v| v <= 0)
        || job.app_server_generation < 1
        || job.channel_id <= 0
        || job.target_thread_id.trim().is_empty()
        || !crate::async_resolution::held_in(db, &job.target_thread_id)?
    {
        return Err(invalid(
            "request is not an original owned, held, unstarted Pending job",
        ));
    }
    let target = Target {
        job: job_id.into(),
        thread: job.target_thread_id,
        owner,
        channel: job.channel_id,
    };
    let mapped: bool = db.query_row(
        "SELECT count(*)=1 AND MAX(codex_thread_id=?1 AND discord_thread_id=?2)
         FROM mirror_threads WHERE codex_thread_id=?1 OR discord_thread_id=?2",
        params![target.thread, target.channel],
        |r| r.get(0),
    )?;
    let unresolved: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_mutation_attempts
         WHERE state='prepared' AND (scoped=0 OR target_thread_id=?1))
         OR EXISTS(SELECT 1 FROM codex_request_cancellations WHERE job_id=?2 OR discord_message_id=?3)",
        params![target.thread,job_id,job.discord_message_id], |r|r.get(0),
    )?;
    if !mapped || unresolved {
        return Err(invalid(
            "mapping or unresolved wire/cancellation authority changed",
        ));
    }
    let mut budget = 0;
    let row = job_row_in(db, job_id, &mut budget)?;
    let context = context_in(db, path, &target, source, creating, &mut budget)?;
    let evidence = json!({"job":row,"context":context});
    if serde_json::to_vec(&evidence)?.len() > MAX_BYTES {
        return Err(invalid("private snapshot exceeds bound"));
    }
    Ok(Captured { target, evidence })
}

pub(super) fn context_in(
    db: &Connection,
    path: &Path,
    target: &Target,
    source: &str,
    creating: bool,
    budget: &mut usize,
) -> Result<Value> {
    let mut context = serde_json::Map::new();
    for (key,sql) in [
        ("siblings","SELECT * FROM codex_turn_queue WHERE target_thread_id=?1 AND job_id!=?2 ORDER BY created_at,job_id LIMIT 129"),
        ("mapping","SELECT * FROM mirror_threads WHERE codex_thread_id=?1 OR discord_thread_id=?3 ORDER BY codex_thread_id LIMIT 3"),
        ("obligations","SELECT * FROM cdr_async_execution_obligations WHERE thread_id=?1 ORDER BY question_id LIMIT 129"),
        ("questions","SELECT * FROM cdr_async_questions WHERE thread_id=?1 ORDER BY id LIMIT 129"),
        ("settlements","SELECT * FROM cdr_async_terminal_settlements WHERE question_id IN
            (SELECT question_id FROM cdr_async_execution_obligations WHERE thread_id=?1) ORDER BY question_id LIMIT 129"),
        ("handoffs","SELECT * FROM cdr_async_execution_handoffs WHERE question_id IN
            (SELECT question_id FROM cdr_async_execution_obligations WHERE thread_id=?1) ORDER BY question_id,revision LIMIT 129"),
        ("terminal_candidates","SELECT * FROM cdr_async_terminal_candidates WHERE question_id IN
            (SELECT question_id FROM cdr_async_execution_obligations WHERE thread_id=?1) ORDER BY question_id,revision,kind,evidence_sha256 LIMIT 129"),
        ("policy","SELECT * FROM cdr_async_recovery_policies WHERE thread_id=?1 LIMIT 2"),
        ("intakes","SELECT * FROM codex_prompt_intakes WHERE target_thread_id=?1 ORDER BY rowid LIMIT 129"),
        ("stop_controls","SELECT * FROM cdr_stop_controls WHERE target_thread_id=?1 ORDER BY sequence LIMIT 129"),
        ("archive","SELECT * FROM codex_archive_fences WHERE target_thread_id=?1 LIMIT 2"),
        ("cleanup","SELECT * FROM cdr_cleanup_fences WHERE target_thread_id=?1 OR channel_id=?3 ORDER BY channel_id LIMIT 129"),
        ("dead_generation","SELECT * FROM codex_dead_generation_holds WHERE target_thread_id=?1 ORDER BY rowid LIMIT 129"),
        ("execution_holds","SELECT * FROM cdr_execution_holds WHERE target_thread_id=?1 ORDER BY job_id LIMIT 129"),
        ("other_cancellations","SELECT * FROM codex_request_cancellations WHERE target_thread_id=?1 AND job_id!=?2 ORDER BY job_id LIMIT 129"),
        ("prepared_wire","SELECT * FROM codex_mutation_attempts WHERE state='prepared' AND (scoped=0 OR target_thread_id=?1) ORDER BY sequence LIMIT 129"),
    ] {
        // Bind only parameters actually used by this fixed statement.
        let mut statement = db.prepare(sql)?;
        statement.raw_bind_parameter(1, &target.thread)?;
        if statement.parameter_count() >= 2 { statement.raw_bind_parameter(2, &target.job)?; }
        if statement.parameter_count() >= 3 { statement.raw_bind_parameter(3, target.channel)?; }
        context.insert(key.into(), rows(&mut statement, budget)?);
    }
    let mut caps = db.prepare(
        "SELECT * FROM cdr_runtime_capability_requirements ORDER BY component LIMIT 129",
    )?;
    context.insert("capabilities".into(), rows(&mut caps, budget)?);
    context.insert(
        "stop_origin".into(),
        crate::ingress::stop::revision::capture_in(db, Some(&target.thread))?,
    );
    context.insert("runtime".into(), identity::runtime_in(db)?);
    context.insert(
        "source".into(),
        identity::message_in(db, target, source, creating)?,
    );
    context.insert("database".into(), json!(database_identity(path)?));
    Ok(Value::Object(context))
}

pub(super) fn job_row_in(db: &Connection, job: &str, budget: &mut usize) -> Result<Value> {
    let mut statement = db.prepare("SELECT * FROM codex_turn_queue WHERE job_id=?")?;
    statement.raw_bind_parameter(1, job)?;
    rows(&mut statement, budget)
}

pub(super) fn rows(statement: &mut rusqlite::Statement<'_>, budget: &mut usize) -> Result<Value> {
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|v| (*v).into())
        .collect();
    let mut rows = statement.raw_query();
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        if values.len() >= MAX_ROWS {
            return Err(invalid("private evidence page exceeds bound"));
        }
        let mut cells = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            let cell = match row.get_ref(index)? {
                ValueRef::Null => json!(["null"]),
                ValueRef::Integer(v) => json!(["integer", v]),
                ValueRef::Real(v) => json!(["real_bits", v.to_bits().to_string()]),
                ValueRef::Text(v) => {
                    if v.len() > MAX_BYTES {
                        return Err(invalid("private evidence cell exceeds bound"));
                    }
                    json!([
                        "text",
                        std::str::from_utf8(v).map_err(|_| invalid("non-UTF8 evidence"))?
                    ])
                }
                ValueRef::Blob(v) => {
                    if v.len() > MAX_BYTES / 2 {
                        return Err(invalid("private evidence blob exceeds bound"));
                    }
                    json!(["blob_hex", hex::encode(v)])
                }
            };
            *budget = budget.saturating_add(serde_json::to_vec(&cell)?.len());
            if *budget > MAX_BYTES {
                return Err(invalid("private evidence exceeds bound"));
            }
            cells.push(cell);
        }
        values.push(cells);
    }
    Ok(json!({"columns":columns,"rows":values}))
}
