use rusqlite::{Transaction, params};
use serde_json::{Value, json};

use super::{store::Row, validation};

pub(super) fn restart(
    tx: &Transaction<'_>,
    scope: &str,
    row: Option<&Row>,
    failed: &str,
    now: i64,
) -> Result<Value, String> {
    let Some(row) = row else {
        return Ok(json!({"status":"missing"}));
    };
    if row.restart {
        if let Some(url) = &row.url {
            return Ok(
                json!({"status":if validation::same(url, failed)? {"exhausted"} else {"superseded"},"url":url}),
            );
        }
        return Ok(json!({"status":if row.busy(now) {"busy"} else {"stalled"}}));
    }
    let Some(url) = &row.url else {
        return Ok(json!({"status":if row.busy(now) {"busy"} else {"missing"}}));
    };
    if !validation::same(url, failed)? {
        return Ok(json!({"status":"superseded","url":url}));
    }
    let token = validation::token();
    tx.execute("UPDATE conversations SET conversation_url=NULL, lease_hash=?, lease_expires_at=?, updated_at=?, restart_pending=1, restart_from_url=? WHERE scope=? AND conversation_url=?", params![validation::hash(&token), now + 120, now, url, scope, url]).map_err(|e| e.to_string())?;
    Ok(json!({"status":"acquired","lease_token":token}))
}

pub(super) fn complete(
    tx: &Transaction<'_>,
    scope: &str,
    row: Option<&Row>,
    expected: &str,
    now: i64,
) -> Result<Value, String> {
    let Some(row) = row else {
        return Ok(json!({"status":"missing"}));
    };
    let Some(url) = &row.url else {
        return Ok(json!({"status":"unavailable"}));
    };
    if !validation::same(url, expected)? {
        return Ok(json!({"status":"superseded","url":url}));
    }
    if !row.restart {
        return Ok(json!({"status":"unchanged"}));
    }
    tx.execute("UPDATE conversations SET restart_pending=0, restart_from_url=NULL, updated_at=? WHERE scope=?", params![now, scope]).map_err(|e| e.to_string())?;
    Ok(json!({"status":"completed"}))
}

pub(super) fn restore(
    tx: &Transaction<'_>,
    scope: &str,
    row: Option<&Row>,
    failed: &str,
    now: i64,
) -> Result<Value, String> {
    let Some(row) = row else {
        return Ok(json!({"status":"missing"}));
    };
    if let Some(url) = &row.url {
        return Ok(json!({"status":"superseded","url":url}));
    }
    let Some(origin) = row.origin.as_ref().filter(|_| row.restart) else {
        return Ok(json!({"status":"missing"}));
    };
    if !validation::same(origin, failed)? {
        return Ok(json!({"status":"superseded","url":origin}));
    }
    if row.busy(now) {
        return Ok(json!({"status":"busy"}));
    }
    tx.execute("UPDATE conversations SET conversation_url=restart_from_url, lease_hash=NULL, lease_expires_at=NULL, updated_at=?, restart_pending=0, restart_from_url=NULL WHERE scope=?", params![now, scope]).map_err(|e| e.to_string())?;
    Ok(json!({"status":"restored","url":origin}))
}

pub(super) fn delete(
    tx: &Transaction<'_>,
    scope: &str,
    row: Option<&Row>,
) -> Result<Value, String> {
    if let Some(row) = row.filter(|row| row.restart) {
        return Ok(row.url.as_ref().map_or_else(
            || json!({"status":"protected"}),
            |url| json!({"status":"protected","url":url}),
        ));
    }
    tx.execute("DELETE FROM conversations WHERE scope=?", [scope])
        .map_err(|e| e.to_string())?;
    Ok(json!({"status":"deleted"}))
}
