//! Bounded, read-only recovery inventory. This is not an apply/authorization API.
mod lifecycle;
use super::args::Args;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

const ROW_LIMIT: usize = 128;
const BYTE_LIMIT: i64 = 131_072;
type InspectResult<T> = Result<T, Box<dyn std::error::Error>>;

pub(super) fn run(args: &Args, root: &Path) -> Result<String, String> {
    let thread = args.required("--thread-id")?;
    if thread.trim().is_empty() || thread.len() > 512 {
        return Err("invalid bounded thread identity".into());
    }
    inspect(&root.join(args.required("--database")?), thread)
        .map(|value| value.to_string())
        .map_err(|error| format!("Cannot inspect async recovery: {error}"))
}

fn table(db: &Connection, name: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?)",
        [name],
        |row| row.get(0),
    )
}

fn hash(value: Option<&str>) -> Option<String> {
    value.map(|value| hex::encode(Sha256::digest(value.as_bytes())))
}

fn page(mut rows: Vec<Value>) -> Value {
    let truncated = rows.len() > ROW_LIMIT;
    rows.truncate(ROW_LIMIT);
    json!({"rows":rows,"truncated":truncated,"row_limit":ROW_LIMIT})
}

fn inspect(path: &Path, thread: &str) -> InspectResult<Value> {
    // Deliberately never call schema::open_initialized, even for a legacy DB.
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.execute_batch("PRAGMA query_only=ON; BEGIN")?;
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != cdr_store::schema::LATEST_STORE_SCHEMA_VERSION {
        return Err(
            format!("store schema {version} is not supported by this read-only inspector").into(),
        );
    }
    let has_questions = table(&db, "cdr_async_questions")?;
    let has_ledger = table(&db, "cdr_async_execution_obligations")?;
    let has_queue = table(&db, "codex_turn_queue")?;
    let questions = if has_questions {
        questions(&db, thread)?
    } else {
        Vec::new()
    };
    let obligations = if has_ledger {
        obligations(&db, thread)?
    } else {
        Vec::new()
    };
    let jobs = if has_queue {
        jobs(&db, thread)?
    } else {
        Vec::new()
    };
    Ok(json!({
        "protocol":"cdr-async-recovery-inspection-v1","read_only":true,
        "thread_id":thread,"schema_version":version,
        "questions_schema_present":has_questions,"ledger_schema_present":has_ledger,
        "queue_schema_present":has_queue,
        "questions":page(questions),"obligations":page(obligations),"jobs":page(jobs),
        "lifecycle_evidence":lifecycle::inspect(&db,thread)?,
        "execution_authorized":false,"receipt_apply_authorized":false,
        "publication_authorized":false,"requires_evidence_review":true,
        "reason":"Inventory only: absence, receipt state, restart, and this report do not prove terminal execution or authorize any replay. Truncated or oversized evidence is incomplete."
    }))
}

fn questions(db: &Connection, thread: &str) -> InspectResult<Vec<Value>> {
    let has_seal: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('cdr_async_questions') WHERE name='preparation_json')",
        [], |row| row.get(0),
    )?;
    let seal = if has_seal { "preparation_json" } else { "NULL" };
    let mut statement = db.prepare(&format!(
        "SELECT id,runtime_id,generation,turn_id,item_id,origin_job_id,channel_id,owner_user_id,
         state,chosen,dispatch_mode,reply_job_id,accepted_turn_id,
         length(CAST({seal} AS BLOB)),CASE WHEN length(CAST({seal} AS BLOB))<=?2 THEN {seal} END
         FROM cdr_async_questions WHERE thread_id=?1 ORDER BY id LIMIT 129"
    ))?;
    Ok(statement.query_map(params![thread,BYTE_LIMIT], |row| {
        let bytes: Option<i64> = row.get(13)?;
        let sealed: Option<String> = row.get(14)?;
        Ok(json!({
            "id":row.get::<_,String>(0)?,"runtime_id":row.get::<_,String>(1)?,
            "generation":row.get::<_,i64>(2)?,"turn_id":row.get::<_,String>(3)?,
            "item_id":row.get::<_,String>(4)?,"origin_job_id":row.get::<_,String>(5)?,
            "channel_id":row.get::<_,i64>(6)?,"owner_user_id":row.get::<_,i64>(7)?,
            "state":row.get::<_,String>(8)?,"chosen":row.get::<_,Option<i64>>(9)?,
            "dispatch_mode":row.get::<_,Option<String>>(10)?,"reply_job_id":row.get::<_,Option<String>>(11)?,
            "accepted_turn_id":row.get::<_,Option<String>>(12)?,
            "seal_bytes":bytes,"seal_oversized":bytes.is_some_and(|n|n>BYTE_LIMIT),
            "seal_sha256":hash(sealed.as_deref())
        }))
    })?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn obligations(db: &Connection, thread: &str) -> InspectResult<Vec<Value>> {
    // Include settled rows as inventory too; do not reuse an admission predicate
    // or claim that a stored label is validated execution authority.
    let mut statement = db.prepare(
        "SELECT question_id,origin_job_id,turn_id,format_version,revision,answer_state,
         execution_state,admission_state,policy,length(CAST(claim_json AS BLOB)),
         CASE WHEN length(CAST(claim_json AS BLOB))<=?2 THEN claim_json END,
         length(CAST(terminal_proof_json AS BLOB)),
         CASE WHEN length(CAST(terminal_proof_json AS BLOB))<=?2 THEN terminal_proof_json END
         FROM cdr_async_execution_obligations WHERE thread_id=?1 ORDER BY question_id LIMIT 129",
    )?;
    Ok(statement.query_map(params![thread,BYTE_LIMIT], |row| {
        let claim: Option<String> = row.get(10)?;
        let proof: Option<String> = row.get(12)?;
        let claim_bytes: Option<i64> = row.get(9)?;
        let proof_bytes: Option<i64> = row.get(11)?;
        Ok(json!({
            "question_id":row.get::<_,String>(0)?,"origin_job_id":row.get::<_,String>(1)?,
            "turn_id":row.get::<_,String>(2)?,"format_version":row.get::<_,i64>(3)?,
            "revision":row.get::<_,i64>(4)?,"answer_state":row.get::<_,String>(5)?,
            "execution_state":row.get::<_,String>(6)?,"admission_state":row.get::<_,String>(7)?,
            "policy":row.get::<_,String>(8)?,"claim_bytes":claim_bytes,
            "claim_sha256":hash(claim.as_deref()),"terminal_evidence_bytes":proof_bytes,
            "terminal_evidence_sha256":hash(proof.as_deref()),
            "evidence_oversized":claim_bytes.is_some_and(|n|n>BYTE_LIMIT)||proof_bytes.is_some_and(|n|n>BYTE_LIMIT),
            "state_labels_are_not_authority":true
        }))
    })?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn jobs(db: &Connection, thread: &str) -> InspectResult<Vec<Value>> {
    let mut statement = db.prepare(
        "SELECT job_id,state,app_server_generation,execution_generation,turn_observation_generation,
         goal_waiting,attempt_count,turn_id,channel_id,owner_user_id,discord_message_id,
         length(CAST(prompt AS BLOB)),CASE WHEN length(CAST(prompt AS BLOB))<=?2 THEN prompt END,
         created_at,updated_at FROM codex_turn_queue WHERE target_thread_id=?1
         ORDER BY created_at,job_id LIMIT 129"
    )?;
    Ok(statement.query_map(params![thread,BYTE_LIMIT], |row| {
        let bytes: i64 = row.get(11)?;
        let prompt: Option<String> = row.get(12)?;
        Ok(json!({
            "job_id":row.get::<_,String>(0)?,"state":row.get::<_,String>(1)?,
            "app_server_generation":row.get::<_,i64>(2)?,"execution_generation":row.get::<_,Option<i64>>(3)?,
            "turn_observation_generation":row.get::<_,Option<i64>>(4)?,"goal_waiting":row.get::<_,bool>(5)?,
            "attempt_count":row.get::<_,i64>(6)?,"turn_id":row.get::<_,Option<String>>(7)?,
            "channel_id":row.get::<_,i64>(8)?,"owner_user_id":row.get::<_,Option<i64>>(9)?,
            "discord_message_id":row.get::<_,Option<i64>>(10)?,"prompt_bytes":bytes,
            "prompt_sha256":hash(prompt.as_deref()),"prompt_oversized":bytes>BYTE_LIMIT,
            "created_at_bits":row.get::<_,f64>(13)?.to_bits(),"updated_at_bits":row.get::<_,f64>(14)?.to_bits()
        }))
    })?.collect::<rusqlite::Result<Vec<_>>>()?)
}
