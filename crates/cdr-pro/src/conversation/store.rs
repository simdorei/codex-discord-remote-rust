use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::validation;

pub(super) struct Row {
    pub url: Option<String>,
    pub lease: Option<String>,
    pub expiry: Option<i64>,
    pub restart: bool,
    pub origin: Option<String>,
}

impl Row {
    pub fn busy(&self, now: i64) -> bool {
        self.lease.as_ref().is_some_and(|hash| !hash.is_empty()) && self.expiry.unwrap_or(0) >= now
    }
}

pub(super) fn database_path() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("SIMDOREI_PRO_CONVERSATION_DB")
        && !value.trim().is_empty()
    {
        return expand_home(value.trim());
    }
    for name in ["LOCALAPPDATA", "XDG_STATE_HOME"] {
        if let Ok(value) = std::env::var(name)
            && !value.trim().is_empty()
        {
            return Ok(PathBuf::from(value).join("simdorei/ask-chatgpt-pro/conversations.sqlite3"));
        }
    }
    Ok(home()?.join(".local/state/simdorei/ask-chatgpt-pro/conversations.sqlite3"))
}

fn home() -> Result<PathBuf, String> {
    ["USERPROFILE", "HOME"]
        .into_iter()
        .find_map(|key| std::env::var_os(key).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .ok_or("could not locate the user home for the conversation store".into())
}

fn expand_home(value: &str) -> Result<PathBuf, String> {
    if value == "~" {
        return home();
    }
    if value.starts_with("~/") || value.starts_with("~\\") {
        return Ok(home()?.join(&value[2..]));
    }
    Ok(PathBuf::from(value))
}

pub(super) fn connect(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut connection = Connection::open(path).map_err(|e| e.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    transaction.execute_batch("CREATE TABLE IF NOT EXISTS conversations (scope TEXT PRIMARY KEY, conversation_url TEXT, lease_hash TEXT, lease_expires_at INTEGER, updated_at INTEGER NOT NULL, restart_pending INTEGER NOT NULL DEFAULT 0, restart_from_url TEXT)").map_err(|e| e.to_string())?;
    let columns = transaction
        .prepare("PRAGMA table_info(conversations)")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if !columns.iter().any(|name| name == "restart_pending") {
        transaction
            .execute_batch(
                "ALTER TABLE conversations ADD COLUMN restart_pending INTEGER NOT NULL DEFAULT 0",
            )
            .map_err(|e| e.to_string())?;
    }
    if !columns.iter().any(|name| name == "restart_from_url") {
        transaction
            .execute_batch("ALTER TABLE conversations ADD COLUMN restart_from_url TEXT")
            .map_err(|e| e.to_string())?;
    }
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(connection)
}

pub(super) fn row(transaction: &Transaction<'_>, scope: &str) -> Result<Option<Row>, String> {
    let record = transaction.query_row("SELECT conversation_url, lease_hash, lease_expires_at, restart_pending, restart_from_url FROM conversations WHERE scope=?", [scope], |row| {
        let flag: i64 = row.get(3)?;
        if ![0, 1].contains(&flag) { return Err(rusqlite::Error::InvalidQuery); }
        Ok(Row { url: row.get(0)?, lease: row.get(1)?, expiry: row.get(2)?, restart: flag == 1, origin: row.get(4)? })
    }).optional().map_err(|e| format!("The conversation store contains invalid data: {e}"))?;
    if let Some(row) = &record {
        if row.restart != row.origin.is_some() {
            return Err("The conversation store contains invalid data: restart guard".into());
        }
        if let Some(url) = &row.url {
            validation::url(url)?;
        }
        if let Some(url) = &row.origin {
            validation::url(url)?;
        }
    }
    Ok(record)
}
