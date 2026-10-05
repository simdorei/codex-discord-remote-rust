//! Durable category/project creation custody, independent of display metadata.
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
mod category;
mod project;
mod schema;
pub use category::{begin_category, bind_category, category_receipt};
pub(crate) use project::{before_project_in, finish_project_in};
pub use project::{begin_project, ensure_adoptable, project_receipt, project_snapshot};
pub(crate) use schema::{migrate_schema, schema_current};

pub type ProjectSnapshot = Vec<(String, i64)>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt {
    kind: String,
    key: String,
    token: String,
    guild: i64,
    parent: Option<i64>,
    expected: String,
    phase: String,
    channel: Option<i64>,
}

impl Receipt {
    pub fn confirmed_channel(&self) -> Result<i64> {
        self.channel
            .filter(|id| *id > 0 && matches!(self.phase.as_str(), "confirmed" | "bound"))
            .ok_or_else(|| {
                invalid("container creation outcome is unknown; creation will not be repeated")
            })
    }

    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.phase == "bound"
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::Integrity(message.into())
}

fn require(value: bool, message: &str) -> Result<()> {
    if value { Ok(()) } else { Err(invalid(message)) }
}

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Receipt> {
    Ok(Receipt {
        kind: row.get(0)?,
        key: row.get(1)?,
        token: row.get(2)?,
        guild: row.get(3)?,
        parent: row.get(4)?,
        expected: row.get(5)?,
        phase: row.get(6)?,
        channel: row.get(7)?,
    })
}

fn load(db: &Connection, kind: &str, key: &str) -> Result<Option<Receipt>> {
    Ok(db
        .query_row(
            "SELECT kind,scope_key,token,guild_id,parent_id,expected_json,phase,channel_id
         FROM cdr_mirror_container_creations WHERE kind=? AND scope_key=?",
            params![kind, key],
            row,
        )
        .optional()?)
}

fn records(db: &Connection) -> Result<Vec<Receipt>> {
    Ok(db
        .prepare(
            "SELECT kind,scope_key,token,guild_id,parent_id,expected_json,phase,channel_id
         FROM cdr_mirror_container_creations ORDER BY kind,scope_key",
        )?
        .query_map([], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn expected(receipt: &Receipt) -> Result<ProjectSnapshot> {
    serde_json::from_str(&receipt.expected)
        .map_err(|_| invalid("invalid container creation mapping snapshot"))
}

fn check_exact(db: &Connection, receipt: &Receipt) -> Result<()> {
    require(
        load(db, &receipt.kind, &receipt.key)?.as_ref() == Some(receipt),
        "container creation custody changed; intent retained",
    )
}

fn check_cleanup(db: &Connection, receipt: &Receipt) -> Result<()> {
    let ids = receipt
        .parent
        .into_iter()
        .chain(receipt.channel)
        .chain(expected(receipt)?.into_iter().map(|(_, id)| id));
    for id in ids {
        let fenced: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM cdr_cleanup_fences f WHERE f.channel_id=?
             OR EXISTS(SELECT 1 FROM mirror_threads t WHERE t.discord_channel_id=?
             AND (f.channel_id=t.discord_thread_id OR f.target_thread_id=t.codex_thread_id)))",
            [id, id],
            |r| r.get(0),
        )?;
        require(!fenced, "container creation is blocked by room cleanup")?;
    }
    Ok(())
}

fn attempt_in(db: &Connection, receipt: &Receipt, previous: Option<&Receipt>) -> Result<()> {
    let changed = if let Some(previous) = previous {
        check_exact(db, previous)?;
        require(
            previous.kind == "category" && previous.is_bound(),
            "only an authoritatively missing bound category may be replaced",
        )?;
        check_cleanup(db, previous)?;
        db.execute(
            "UPDATE cdr_mirror_container_creations SET token=?,phase='attempted',channel_id=NULL
             WHERE kind='category' AND scope_key=? AND token=? AND phase='bound'",
            params![receipt.token, receipt.key, previous.token],
        )?
    } else {
        require(
            load(db, &receipt.kind, &receipt.key)?.is_none(),
            "container creation is already claimed",
        )?;
        db.execute(
            "INSERT INTO cdr_mirror_container_creations
             (kind,scope_key,token,guild_id,parent_id,expected_json,phase,channel_id)
             VALUES (?,?,?,?,?,?,'attempted',NULL)",
            params![
                receipt.kind,
                receipt.key,
                receipt.token,
                receipt.guild,
                receipt.parent,
                receipt.expected
            ],
        )?
    };
    require(changed == 1, "container creation intent was not stored")?;
    check_exact(db, receipt)?;
    check_cleanup(db, receipt)
}

/// Persist the exact returned ID before any mapping/binding commit; ambiguous writes stay held.
pub fn confirm(path: &Path, receipt: &Receipt, channel: i64) -> Result<Receipt> {
    require(
        channel > 0 && receipt.phase == "attempted" && receipt.channel.is_none(),
        "invalid container creation confirmation",
    )?;
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_exact(&tx, receipt)?;
    let mut confirmed = receipt.clone();
    confirmed.phase = "confirmed".into();
    confirmed.channel = Some(channel);
    let changed = tx.execute(
        "UPDATE cdr_mirror_container_creations SET phase='confirmed',channel_id=?
         WHERE kind=? AND scope_key=? AND token=? AND phase='attempted' AND channel_id IS NULL",
        params![channel, receipt.kind, receipt.key, receipt.token],
    )?;
    require(
        changed == 1,
        "container creation confirmation was not stored",
    )?;
    check_exact(&tx, &confirmed)?;
    tx.commit()?;
    Ok(confirmed)
}

pub(crate) fn protect_cleanup_in(
    db: &Connection,
    channel: i64,
    target: Option<&str>,
) -> Result<()> {
    let mapped_parent: Option<i64> = db
        .query_row(
            "SELECT discord_channel_id FROM mirror_threads WHERE codex_thread_id=?",
            [target],
            |r| r.get(0),
        )
        .optional()?;
    for receipt in records(db)? {
        let previous = expected(&receipt)?;
        let protected = receipt.parent == Some(channel)
            || receipt.channel == Some(channel)
            || previous
                .iter()
                .any(|(_, id)| *id == channel || Some(*id) == mapped_parent)
            || (target.is_none() && receipt.channel.is_none());
        if protected {
            return Err(StoreError::CleanupProtected {
                channel,
                reason: "unresolved mirror container creation",
            });
        }
    }
    Ok(())
}
