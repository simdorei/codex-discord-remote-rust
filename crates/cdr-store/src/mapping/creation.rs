//! Exact thread-room creation custody across cancellation and mapping write failure.
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
mod schema;
pub(crate) use schema::{migrate_schema, schema_current};

#[derive(Clone, Copy, Debug)]
pub struct CreationScope<'a> {
    pub thread: &'a str,
    pub guild: i64,
    pub parent: i64,
    pub expected: Option<(i64, i64)>,
}

#[derive(Eq, PartialEq)]
pub(super) struct Record {
    token: String,
    guild: i64,
    parent: i64,
    expected_parent: Option<i64>,
    expected_channel: Option<i64>,
    phase: String,
    channel: Option<i64>,
}

fn load(db: &Connection, thread: &str) -> Result<Option<Record>> {
    Ok(db.query_row(
        "SELECT token,guild_id,parent_id,expected_parent_id,expected_channel_id,phase,channel_id
         FROM cdr_mirror_thread_creations WHERE thread_id=?",
        [thread],
        |row| Ok(Record {
            token: row.get(0)?, guild: row.get(1)?, parent: row.get(2)?,
            expected_parent: row.get(3)?, expected_channel: row.get(4)?,
            phase: row.get(5)?, channel: row.get(6)?,
        }),
    ).optional()?)
}

fn columns(expected: Option<(i64, i64)>) -> (Option<i64>, Option<i64>) {
    expected.map_or((None, None), |(parent, channel)| {
        (Some(parent), Some(channel))
    })
}

fn check_scope(record: &Record, scope: CreationScope<'_>) -> Result<()> {
    if record.guild != scope.guild
        || record.parent != scope.parent
        || (record.expected_parent, record.expected_channel) != columns(scope.expected)
    {
        return Err(StoreError::Integrity(
            "mirror creation scope changed; intent retained".into(),
        ));
    }
    Ok(())
}

fn mapped(db: &Connection, thread: &str) -> Result<Option<(i64, i64)>> {
    Ok(db.query_row(
        "SELECT discord_channel_id,discord_thread_id FROM mirror_threads WHERE codex_thread_id=?",
        [thread], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?)
}

fn check_cleanup(db: &Connection, scope: CreationScope<'_>) -> Result<()> {
    let blocked: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_cleanup_fences
         WHERE channel_id=? OR channel_id=? OR target_thread_id=?)",
        params![scope.parent, scope.expected.map(|(_, id)| id), scope.thread],
        |row| row.get(0),
    )?;
    if blocked {
        return Err(StoreError::Integrity(
            "mirror creation is blocked by room cleanup".into(),
        ));
    }
    Ok(())
}

/// Unknown custody is an error even if an old mapping happens to become visible again.
pub fn confirmed(path: &Path, scope: CreationScope<'_>) -> Result<Option<i64>> {
    let db = open_initialized(path)?;
    let tx = db.unchecked_transaction()?;
    let Some(record) = load(&tx, scope.thread)? else {
        return Ok(None);
    };
    check_scope(&record, scope)?;
    check_cleanup(&tx, scope)?;
    if mapped(&tx, scope.thread)? != scope.expected {
        return Err(StoreError::Integrity(
            "mirror mapping changed during creation".into(),
        ));
    }
    match (record.phase.as_str(), record.channel) {
        ("confirmed", Some(id)) if id > 0 => Ok(Some(id)),
        _ => Err(StoreError::Integrity(format!(
            "mirror creation outcome is unknown for {}; creation will not be repeated",
            scope.thread
        ))),
    }
}

pub fn begin(path: &Path, scope: CreationScope<'_>) -> Result<String> {
    if scope.thread.trim().is_empty()
        || scope.guild <= 0
        || scope.parent <= 0
        || scope
            .expected
            .is_some_and(|(parent, channel)| parent <= 0 || channel <= 0)
    {
        return Err(StoreError::Integrity(
            "invalid mirror creation scope".into(),
        ));
    }
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if load(&tx, scope.thread)?.is_some() || mapped(&tx, scope.thread)? != scope.expected {
        return Err(StoreError::Integrity(
            "mirror creation already claimed or mapping changed".into(),
        ));
    }
    check_cleanup(&tx, scope)?;
    let token = uuid::Uuid::new_v4().to_string();
    let (parent, channel) = columns(scope.expected);
    let inserted = tx.execute(
        "INSERT INTO cdr_mirror_thread_creations
         (thread_id,token,guild_id,parent_id,expected_parent_id,expected_channel_id,phase)
         VALUES (?,?,?,?,?,?,'attempted')",
        params![
            scope.thread,
            token,
            scope.guild,
            scope.parent,
            parent,
            channel
        ],
    )?;
    let record = load(&tx, scope.thread)?
        .ok_or_else(|| StoreError::Integrity("mirror creation intent was not stored".into()))?;
    check_scope(&record, scope)?;
    if inserted != 1
        || record.token != token
        || record.phase != "attempted"
        || record.channel.is_some()
    {
        return Err(StoreError::Integrity(
            "mirror creation intent write was not exact".into(),
        ));
    }
    tx.commit()?;
    Ok(token)
}

/// A returned ID is retained before mapping is attempted; it is not yet mapping authority.
pub fn confirm(path: &Path, scope: CreationScope<'_>, token: &str, channel: i64) -> Result<()> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let record = load(&tx, scope.thread)?
        .ok_or_else(|| StoreError::Integrity("mirror creation intent disappeared".into()))?;
    check_scope(&record, scope)?;
    if channel <= 0
        || record.token != token
        || record.phase != "attempted"
        || record.channel.is_some()
    {
        return Err(StoreError::Integrity(
            "mirror creation confirmation lost ownership".into(),
        ));
    }
    let changed = tx.execute(
        "UPDATE cdr_mirror_thread_creations SET phase='confirmed',channel_id=?
         WHERE thread_id=? AND token=? AND phase='attempted' AND channel_id IS NULL",
        params![channel, scope.thread, token],
    )?;
    let record = load(&tx, scope.thread)?
        .ok_or_else(|| StoreError::Integrity("mirror creation confirmation disappeared".into()))?;
    check_scope(&record, scope)?;
    if changed != 1
        || record.token != token
        || record.phase != "confirmed"
        || record.channel != Some(channel)
    {
        return Err(StoreError::Integrity(
            "mirror creation confirmation was not stored".into(),
        ));
    }
    tx.commit()?;
    Ok(())
}

/// Pin absence or exact custody before the mapping write can invoke triggers.
pub(super) fn before_mapping_in(db: &Connection, thread: &str) -> Result<Option<Record>> {
    load(db, thread)
}

/// Called in the same transaction as the exact mapping CAS, never after its commit.
pub(super) fn finish_in(
    db: &Connection,
    update: super::MirrorThreadUpdate<'_>,
    expected: Option<(i64, i64)>,
    original: Option<Record>,
) -> Result<()> {
    if load(db, update.thread_id)? != original {
        return Err(StoreError::Integrity(
            "mirror creation custody changed during mapping write".into(),
        ));
    }
    let Some(record) = original else {
        return Ok(());
    };
    let scope = CreationScope {
        thread: update.thread_id,
        guild: record.guild,
        parent: update.parent_id,
        expected,
    };
    check_scope(&record, scope)?;
    check_cleanup(db, scope)?;
    if record.phase != "confirmed"
        || record.channel != Some(update.channel_id)
        || mapped(db, update.thread_id)? != Some((update.parent_id, update.channel_id))
    {
        return Err(StoreError::Integrity(
            "mirror creation mapping was not confirmed".into(),
        ));
    }
    let changed = db.execute(
        "DELETE FROM cdr_mirror_thread_creations WHERE thread_id=? AND token=? AND phase='confirmed' AND channel_id=?",
        params![update.thread_id, record.token, update.channel_id],
    )?;
    if changed != 1
        || load(db, update.thread_id)?.is_some()
        || mapped(db, update.thread_id)? != Some((update.parent_id, update.channel_id))
    {
        return Err(StoreError::Integrity(
            "mirror creation completion was not stored".into(),
        ));
    }
    check_cleanup(db, scope)?;
    Ok(())
}

pub(crate) fn protect_cleanup_in(
    db: &Connection,
    channel: i64,
    target: Option<&str>,
) -> Result<()> {
    let protected: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_mirror_thread_creations
         WHERE thread_id=? OR parent_id=? OR channel_id=? OR (? IS NULL AND channel_id IS NULL))",
        params![target, channel, channel, target],
        |row| row.get(0),
    )?;
    if protected {
        return Err(StoreError::CleanupProtected {
            channel,
            reason: "unresolved mirror creation",
        });
    }
    Ok(())
}
