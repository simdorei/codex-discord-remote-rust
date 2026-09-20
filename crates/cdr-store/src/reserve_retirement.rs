//! One-time, fail-closed retirement of automatic Reserve before executable recovery.
use crate::{Result, StoreError, execution_hold, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

const VERSION: &str = "manual-reserve-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadlessEvidence {
    pub job_id: String,
    pub row_sha256: String,
    pub provenance: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Retirement {
    pub already_completed: bool,
    pub held_jobs: usize,
    pub policy_snapshots: usize,
}

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cdr_store_retirements (
        name TEXT PRIMARY KEY, evidence_json TEXT NOT NULL, completed_at REAL NOT NULL);
        CREATE TABLE IF NOT EXISTS cdr_reserve_retirement_evidence (
        thread_id TEXT PRIMARY KEY, policy_json TEXT NOT NULL, retired_at REAL NOT NULL);",
    )?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT COUNT(*)=2 FROM sqlite_schema WHERE type='table'
        AND name IN ('cdr_store_retirements','cdr_reserve_retirement_evidence')",
        [],
        |r| r.get(0),
    )?)
}

/// Caller owns the exclusive runtime/cutover guard; no dispatcher may run before this commits.
pub fn retire(path: &Path, headless: &[HeadlessEvidence]) -> Result<Retirement> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if tx
        .query_row(
            "SELECT 1 FROM cdr_store_retirements WHERE name=?",
            [VERSION],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Ok(Retirement {
            already_completed: true,
            ..Default::default()
        });
    }
    let mut result = Retirement::default();
    let policies = rows(&tx, "SELECT * FROM codex_reserve_policy ORDER BY thread_id")?;
    for policy in policies {
        let thread = policy["thread_id"]
            .as_str()
            .ok_or_else(|| invalid("missing policy thread"))?;
        result.policy_snapshots += tx.execute(
            "INSERT INTO cdr_reserve_retirement_evidence
            (thread_id,policy_json,retired_at) VALUES (?,?,unixepoch())",
            params![thread, policy.to_string()],
        )?;
    }
    // Only mode loses authority. Unresolved episodes, revisions and observations survive intact.
    tx.execute(
        "UPDATE codex_reserve_policy SET mode='manual' WHERE mode!='manual'",
        [],
    )?;
    let mut used_headless = std::collections::BTreeSet::new();
    for (sql, intake) in [
        (
            "SELECT * FROM codex_turn_queue WHERE state='pending' ORDER BY job_id",
            false,
        ),
        ("SELECT * FROM codex_prompt_intakes ORDER BY job_id", true),
    ] {
        for row in rows(&tx, sql)? {
            let job = row["job_id"]
                .as_str()
                .ok_or_else(|| invalid("missing job id"))?;
            let target = row["target_thread_id"]
                .as_str()
                .ok_or_else(|| invalid("missing target"))?;
            let ingress = rows_for_job(&tx, job)?;
            let confirmed = ingress.len() == 1
                && ingress[0]["confirmation_delivered"] == 1
                && ingress[0]["target_thread_id"] == row["target_thread_id"]
                && ingress[0]["channel_id"] == row["channel_id"]
                && ingress[0]["owner_user_id"] == row["owner_user_id"]
                && matches!(ingress[0]["state"].as_str(), Some("owned" | "completed"));
            let failure = row["last_error"].as_str().unwrap_or("");
            let error_reported = confirmed_error_for_ingress(&tx, &ingress)?;
            let override_row = headless.iter().find(|e| e.job_id == job);
            let headless_confirmed = if let Some(e) = override_row {
                if intake
                    || !ingress.is_empty()
                    || !row["discord_message_id"].is_null()
                    || e.provenance.trim().is_empty()
                    || e.row_sha256 != row_hash(&row)
                {
                    return Err(invalid(
                        "headless evidence does not match an unlinked exact queue row",
                    ));
                }
                used_headless.insert(job.to_owned());
                true
            } else {
                false
            };
            if !failure.trim().is_empty() || error_reported || (!confirmed && !headless_confirmed) {
                execution_hold::hold_in(
                    &tx,
                    job,
                    target,
                    "Legacy request requires explicit recovery; changing the model will not execute it",
                    &json!({"row":row,"ingress":ingress,"intake":intake}).to_string(),
                )?;
                result.held_jobs += 1;
            }
        }
    }
    if used_headless.len() != headless.len() {
        return Err(invalid("unused or duplicate headless classification"));
    }
    crate::ingress::retire_unowned_in(&tx)?;
    tx.execute("INSERT INTO cdr_store_retirements(name,evidence_json,completed_at) VALUES (?,?,unixepoch())",
        params![VERSION,json!({"result":result,"headless":headless}).to_string()])?;
    tx.commit()?;
    Ok(result)
}

#[must_use]
pub fn row_hash(row: &Value) -> String {
    hex::encode(Sha256::digest(row.to_string().as_bytes()))
}

fn rows_for_job(db: &Connection, job: &str) -> Result<Vec<Value>> {
    let mut statement = db.prepare("SELECT * FROM discord_ingress_journal WHERE owner_kind='prompt' AND owner_id=? AND kind IN ('message','interaction') ORDER BY created_at,ingress_id")?;
    values(&mut statement, [job])
}

fn confirmed_error_for_ingress(db: &Connection, ingress: &[Value]) -> Result<bool> {
    for i in ingress {
        if i["kind"] != "message" {
            continue;
        }
        let Some(event) = i["event_id"].as_i64() else {
            continue;
        };
        let key = json!([
            i["channel_id"],
            "message/error/v1",
            format!("inbound-message/{event}/error-report"),
            0
        ])
        .to_string();
        if db.query_row("SELECT EXISTS(SELECT 1 FROM codex_delivery_receipts WHERE receipt_key=? AND message_id IS NOT NULL)", [key], |r| r.get::<_,bool>(0))? { return Ok(true); }
    }
    Ok(false)
}

pub(crate) fn rows(db: &Connection, sql: &str) -> Result<Vec<Value>> {
    values(&mut db.prepare(sql)?, [])
}

fn values<P: rusqlite::Params>(
    statement: &mut rusqlite::Statement<'_>,
    params: P,
) -> Result<Vec<Value>> {
    use rusqlite::types::ValueRef;
    let names: Vec<_> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    Ok(statement
        .query_map(params, |row| {
            let mut value = serde_json::Map::new();
            for (i, name) in names.iter().enumerate() {
                value.insert(
                    name.clone(),
                    match row.get_ref(i)? {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(v) => json!(v),
                        ValueRef::Real(v) => json!(v),
                        ValueRef::Text(v) => json!(String::from_utf8_lossy(v)),
                        ValueRef::Blob(v) => json!({"blob_hex":hex::encode(v)}),
                    },
                );
            }
            Ok(Value::Object(value))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn invalid(message: &str) -> StoreError {
    StoreError::Integrity(message.into())
}

#[cfg(test)]
mod tests;
