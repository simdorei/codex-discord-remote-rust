use rusqlite::{Transaction, params};
use serde_json::{Value, json};

use super::{store::Row, validation};

pub(super) fn status(row: Option<&Row>, now: i64) -> Value {
    let Some(row) = row else {
        return json!({"status":"missing"});
    };
    if let Some(url) = &row.url {
        return json!({"status":"found","url":url});
    }
    if row.busy(now) {
        return json!({"status":"busy"});
    }
    if row.restart {
        return json!({"status":"stalled"});
    }
    json!({"status":"missing"})
}

pub(super) fn acquire(
    tx: &Transaction<'_>,
    scope: &str,
    row: Option<&Row>,
    now: i64,
) -> Result<Value, String> {
    let existing = status(row, now);
    if existing["status"] != "missing" {
        return Ok(existing);
    }
    let token = validation::token();
    tx.execute("INSERT INTO conversations(scope, conversation_url, lease_hash, lease_expires_at, updated_at) VALUES (?, NULL, ?, ?, ?) ON CONFLICT(scope) DO UPDATE SET lease_hash=excluded.lease_hash, lease_expires_at=excluded.lease_expires_at, updated_at=excluded.updated_at", params![scope, validation::hash(&token), now + 120, now]).map_err(|e| e.to_string())?;
    Ok(json!({"status":"acquired","lease_token":token}))
}

pub(super) fn save(
    tx: &Transaction<'_>,
    scope: &str,
    row: Option<&Row>,
    url: &str,
    token: &str,
    now: i64,
) -> Result<Value, String> {
    let row = row
        .filter(|row| validation::owns(row.lease.as_deref(), token))
        .ok_or("The conversation creation lease is missing or was replaced.")?;
    if let Some(origin) = &row.origin
        && row.restart
        && validation::same(url, origin)?
    {
        return Err(
            "The replacement conversation must differ from the failed conversation.".into(),
        );
    }
    tx.execute("UPDATE conversations SET conversation_url=?, lease_hash=NULL, lease_expires_at=NULL, updated_at=? WHERE scope=?", params![url, now, scope]).map_err(|e| e.to_string())?;
    Ok(json!({"status":"saved"}))
}

pub(super) fn release(
    tx: &Transaction<'_>,
    scope: &str,
    row: Option<&Row>,
    token: &str,
    now: i64,
) -> Result<Value, String> {
    if let Some(row) = row.filter(|row| validation::owns(row.lease.as_deref(), token)) {
        if row.url.is_some() {
            tx.execute(
                "UPDATE conversations SET lease_hash=NULL, lease_expires_at=NULL WHERE scope=?",
                [scope],
            )
            .map_err(|e| e.to_string())?;
        } else if row.restart && row.origin.is_some() {
            tx.execute("UPDATE conversations SET conversation_url=restart_from_url, lease_hash=NULL, lease_expires_at=NULL, updated_at=?, restart_pending=0, restart_from_url=NULL WHERE scope=?", params![now, scope]).map_err(|e| e.to_string())?;
        } else {
            tx.execute("DELETE FROM conversations WHERE scope=?", [scope])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(json!({"status":"released"}))
}
