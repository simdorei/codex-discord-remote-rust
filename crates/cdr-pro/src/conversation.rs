//! Durable Pro conversation leases, using the existing on-disk `SQLite` schema.
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::TransactionBehavior;
use serde_json::Value;

mod creation;
mod recovery;
mod store;
mod validation;

pub fn run(arguments: impl Iterator<Item = String>) -> Result<Value, String> {
    let mut args = arguments;
    let action = args.next().ok_or("missing conversation command")?;
    let allowed: &[&str] = match action.as_str() {
        "acquire" | "delete" | "status" => &[],
        "set" => &["--url", "--lease-token"],
        "release" => &["--lease-token"],
        "restart" | "restore-stalled" => &["--failed-url"],
        "complete-restart" => &["--url"],
        _ => return Err("unknown conversation command".into()),
    };
    let mut values = BTreeMap::new();
    while let Some(key) = args.next() {
        if key != "--scope" && !allowed.contains(&key.as_str()) {
            return Err("unknown conversation option".into());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value after {key}"))?;
        if values.insert(key, value).is_some() {
            return Err("duplicate conversation option".into());
        }
    }
    let required = |key: &str| -> Result<&str, String> {
        values
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {key}"))
    };
    let scope = required("--scope")?;
    for name in allowed {
        required(name)?;
    }
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs(),
    )
    .map_err(|e| e.to_string())?;
    execute(
        &store::database_path()?,
        &action,
        scope,
        values
            .get("--url")
            .or_else(|| values.get("--failed-url"))
            .map(String::as_str),
        values.get("--lease-token").map(String::as_str),
        now,
    )
}

pub fn execute(
    path: &Path,
    action: &str,
    scope: &str,
    url: Option<&str>,
    lease: Option<&str>,
    now: i64,
) -> Result<Value, String> {
    validation::scope(scope)?;
    if let Some(url) = url {
        validation::url(url)?;
    }
    if let Some(lease) = lease {
        validation::lease(lease)?;
    }
    let mut connection = store::connect(path)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let row = store::row(&transaction, scope)?;
    let output = match action {
        "status" => creation::status(row.as_ref(), now),
        "acquire" => creation::acquire(&transaction, scope, row.as_ref(), now)?,
        "set" => creation::save(
            &transaction,
            scope,
            row.as_ref(),
            url.ok_or("missing URL")?,
            lease.ok_or("missing lease")?,
            now,
        )?,
        "release" => creation::release(
            &transaction,
            scope,
            row.as_ref(),
            lease.ok_or("missing lease")?,
            now,
        )?,
        "restart" => recovery::restart(
            &transaction,
            scope,
            row.as_ref(),
            url.ok_or("missing failed URL")?,
            now,
        )?,
        "complete-restart" => recovery::complete(
            &transaction,
            scope,
            row.as_ref(),
            url.ok_or("missing URL")?,
            now,
        )?,
        "restore-stalled" => recovery::restore(
            &transaction,
            scope,
            row.as_ref(),
            url.ok_or("missing failed URL")?,
            now,
        )?,
        "delete" => recovery::delete(&transaction, scope, row.as_ref())?,
        _ => return Err("unknown conversation command".into()),
    };
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(output)
}
