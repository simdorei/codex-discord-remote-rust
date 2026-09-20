//! Explicit local authorization for one already-generated final, never for execution.
use crate::{Result, StoreError, delivery::StoredDelivery, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const EXPLANATION: &str = "[저장된 답변 복구]\n앞서 오류가 안내됐지만 이후 생성되어 저장된 답변을 전달합니다. 요청을 다시 실행하지 않았습니다.\n\n";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub delivery_id: String,
    pub job_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub channel_id: i64,
    pub original_sha256: String,
    pub ingress_id: String,
    pub error_receipt_key: String,
    pub error_message_id: String,
    pub error_sha256: String,
}

#[derive(Serialize, Deserialize)]
struct Grant {
    request: Request,
    content_sha256: String,
    chunks: Vec<String>,
    ingress: Value,
}

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cdr_final_recovery (
        delivery_id TEXT PRIMARY KEY, grant_json TEXT NOT NULL, created_at REAL NOT NULL);",
    )?;
    Ok(())
}
pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT COUNT(*)=3 FROM pragma_table_info('cdr_final_recovery')",
        [],
        |r| r.get(0),
    )?)
}
#[must_use]
pub fn sha256(content: &str) -> String {
    hex::encode(Sha256::digest(content.as_bytes()))
}

/// The trusted runtime renderer supplies the exact normal completion chunks.
/// Writer-side validation later compares each actual chunk with this frozen manifest.
pub fn authorize(
    path: &Path,
    expected: &Request,
    render: impl FnOnce(&str) -> Vec<String>,
) -> Result<()> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let pending = crate::delivery::select(&tx, &expected.delivery_id)?;
    if let Some(grant) = read(&tx, &expected.delivery_id)? {
        if &grant.request != expected {
            return Err(invalid(
                "recovery authorization differs from its existing grant",
            ));
        }
        validate(&tx, &pending, &grant)?;
        return Ok(());
    }
    require_identity(&pending, expected)?;
    if sha256(&pending.content) != expected.original_sha256 {
        return Err(invalid("saved final payload changed"));
    }
    let ingress = evidence(&tx, expected)?;
    // Scan every existing identity for this completion, including stale destinations
    // and chunks beyond the current rendering. Absence is not inferred from attempts.
    let any_intent: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_delivery_receipts
        WHERE json_valid(receipt_key) AND json_extract(receipt_key,'$[1]')='completion/v1'
        AND json_extract(receipt_key,'$[2]')=?)",
        [&expected.delivery_id],
        |r| r.get(0),
    )?;
    if any_intent {
        return Err(invalid(
            "saved final already has output intent or receipt; no new authorization",
        ));
    }
    require_progress_clear(&tx, &pending)?;
    let content = format!("{EXPLANATION}{}", pending.content);
    let chunks = render(&content);
    if chunks.is_empty()
        || chunks
            .iter()
            .any(|c| c.is_empty() || c.chars().count() > 2000)
    {
        return Err(invalid("invalid completion chunk manifest"));
    }
    let grant = Grant {
        request: expected.clone(),
        content_sha256: sha256(&content),
        chunks: chunks.iter().map(|c| sha256(c)).collect(),
        ingress,
    };
    tx.execute(
        "UPDATE codex_delivery_outbox SET content=? WHERE delivery_id=? AND content=?",
        params![content, expected.delivery_id, pending.content],
    )?;
    tx.execute("INSERT INTO cdr_final_recovery(delivery_id,grant_json,created_at) VALUES (?,?,unixepoch())",
        params![expected.delivery_id,serde_json::to_string(&grant)?])?;
    tx.commit()?;
    Ok(())
}

pub fn authorized(path: &Path, pending: &StoredDelivery) -> Result<bool> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let Some(grant) = read(&tx, &pending.delivery_id)? else {
        return Ok(false);
    };
    validate(&tx, pending, &grant)?;
    Ok(true)
}

/// Called under the receipt writer transaction before the send-intent claim.
pub(crate) fn validate_claim_in(
    db: &Connection,
    key: &str,
    hash: &str,
    guard: Option<&crate::new_reply::DeliveryGuard<'_>>,
) -> Result<()> {
    let Ok((channel, domain, id, index)) =
        serde_json::from_str::<(i64, String, String, usize)>(key)
    else {
        return Ok(());
    };
    if domain != "completion/v1" {
        return Ok(());
    }
    if let Some(grant) = read(db, &id)? {
        let pending = crate::delivery::select(db, &id)?;
        validate(db, &pending, &grant)?;
        let matched = guard.is_some_and(|g| {
            g.job_id == pending.job_id
                && g.thread_id == pending.target_thread_id
                && g.turn_id == pending.turn_id
        });
        if !matched
            || channel != pending.channel_id
            || grant.chunks.get(index).map(String::as_str) != Some(hash)
        {
            return Err(invalid("recovery chunk identity or payload changed"));
        }
    }
    Ok(())
}

fn read(db: &Connection, id: &str) -> Result<Option<Grant>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT grant_json FROM cdr_final_recovery WHERE delivery_id=?",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
fn require_identity(p: &StoredDelivery, r: &Request) -> Result<()> {
    if p.delivery_id != r.delivery_id
        || p.job_id != r.job_id
        || p.target_thread_id != r.thread_id
        || p.turn_id != r.turn_id
        || p.channel_id != r.channel_id
    {
        return Err(invalid("saved final owner or destination changed"));
    }
    Ok(())
}
fn validate(db: &Connection, p: &StoredDelivery, g: &Grant) -> Result<()> {
    require_identity(p, &g.request)?;
    let actual = crate::delivery::select(db, &p.delivery_id)?;
    require_identity(&actual, &g.request)?;
    if sha256(&p.content) != g.content_sha256
        || sha256(&actual.content) != g.content_sha256
        || evidence(db, &g.request)? != g.ingress
    {
        return Err(invalid("recovery evidence or frozen payload changed"));
    }
    require_progress_clear(db, p)
}
fn evidence(db: &Connection, r: &Request) -> Result<Value> {
    let mut stmt = db.prepare(
        "SELECT ingress_id,kind,event_id,channel_id,owner_user_id,target_thread_id,
        state,confirmation_delivered,canonical_owner,payload_json FROM discord_ingress_journal
        WHERE owner_kind='prompt' AND owner_id=? ORDER BY created_at,ingress_id",
    )?;
    let owners=stmt.query_map([&r.job_id],|row| Ok(json!({
        "id":row.get::<_,String>(0)?,"kind":row.get::<_,String>(1)?,"event":row.get::<_,Option<i64>>(2)?,
        "channel":row.get::<_,i64>(3)?,"actor":row.get::<_,i64>(4)?,"thread":row.get::<_,Option<String>>(5)?,
        "state":row.get::<_,String>(6)?,"confirmed":row.get::<_,bool>(7)?,
        "canonical":row.get::<_,Option<String>>(8)?,"payload":row.get::<_,String>(9)?
    })))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if owners.len() != 1 {
        return Err(invalid(
            "recovery requires one unambiguous canonical ingress",
        ));
    }
    let i = &owners[0];
    let event = i["event"]
        .as_i64()
        .ok_or_else(|| invalid("recovery source event missing"))?;
    let key = json!([
        r.channel_id,
        "message/error/v1",
        format!("inbound-message/{event}/error-report"),
        0
    ])
    .to_string();
    if i["id"] != r.ingress_id
        || i["kind"] != "message"
        || i["channel"] != r.channel_id
        || i["thread"] != r.thread_id
        || i["confirmed"] != false
        || !matches!(i["state"].as_str(), Some("owned" | "completed"))
        || key != r.error_receipt_key
    {
        return Err(invalid(
            "recovery source is not this exact failed original request",
        ));
    }
    let receipt: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT content_hash,message_id FROM codex_delivery_receipts WHERE receipt_key=?",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if receipt != Some((r.error_sha256.clone(), Some(r.error_message_id.clone())))
        || r.error_message_id.is_empty()
    {
        return Err(invalid("original error report is not exactly confirmed"));
    }
    let mut stmt =
        db.prepare("SELECT codex_thread_id FROM mirror_threads WHERE discord_thread_id=?")?;
    let mapping = stmt
        .query_map([r.channel_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if mapping != vec![r.thread_id.clone()] {
        return Err(invalid("recovery destination no longer owns this thread"));
    }
    let executable: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_turn_queue WHERE job_id=?)",
        [&r.job_id],
        |row| row.get(0),
    )?;
    if executable {
        return Err(invalid(
            "original job still has executable or uncertain queue custody",
        ));
    }
    Ok(i.clone())
}
fn require_progress_clear(db: &Connection, p: &StoredDelivery) -> Result<()> {
    let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM codex_commentary_outbox WHERE job_id=?1)
        OR EXISTS(SELECT 1 FROM codex_goal_progress WHERE job_id=?1 OR (job_id IS NULL AND thread=?2))",
        params![p.job_id,p.target_thread_id],|r|r.get(0))?;
    if pending {
        return Err(invalid("saved final held behind undelivered progress"));
    }
    Ok(())
}
fn invalid(s: &str) -> StoreError {
    StoreError::Integrity(s.into())
}

#[cfg(test)]
mod tests;
