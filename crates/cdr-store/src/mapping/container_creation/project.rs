use super::{
    ProjectSnapshot, Receipt, attempt_in, check_cleanup, expected, invalid, records, require,
};
use crate::{Result, schema::open_initialized};
use rusqlite::{Connection, TransactionBehavior, params};
use std::path::Path;

fn snapshot(
    db: &Connection,
    key: &str,
    matches: &impl Fn(&str, &str) -> bool,
) -> Result<ProjectSnapshot> {
    Ok(db
        .prepare("SELECT project_key,discord_channel_id FROM mirror_projects ORDER BY project_key")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(stored, _)| matches(stored, key))
        .collect())
}

fn find(
    db: &Connection,
    key: &str,
    matches: &impl Fn(&str, &str) -> bool,
) -> Result<Option<Receipt>> {
    let mut found = records(db)?
        .into_iter()
        .filter(|r| r.kind == "project" && matches(&r.key, key));
    let first = found.next();
    require(
        found.next().is_none(),
        "multiple project creation claims; reconciliation required",
    )?;
    Ok(first)
}

pub fn project_snapshot(
    path: &Path,
    key: &str,
    matches: impl Fn(&str, &str) -> bool,
) -> Result<ProjectSnapshot> {
    let result = snapshot(&open_initialized(path)?, key, &matches)?;
    require(
        result
            .iter()
            .all(|(_, id)| *id > 0 && Some(id) == result.first().map(|(_, id)| id)),
        "project aliases have conflicting channel identities",
    )?;
    Ok(result)
}

pub fn project_receipt(
    path: &Path,
    key: &str,
    guild: i64,
    parent: i64,
    original: &ProjectSnapshot,
    matches: impl Fn(&str, &str) -> bool,
) -> Result<Option<Receipt>> {
    let db = open_initialized(path)?;
    let tx = db.unchecked_transaction()?;
    let Some(receipt) = find(&tx, key, &matches)? else {
        return Ok(None);
    };
    require(
        receipt.guild == guild
            && receipt.parent == Some(parent)
            && expected(&receipt)? == *original
            && snapshot(&tx, key, &matches)? == *original,
        "project creation scope or mapping changed",
    )?;
    receipt.confirmed_channel()?;
    check_cleanup(&tx, &receipt)?;
    Ok(Some(receipt))
}

pub fn begin_project(
    path: &Path,
    key: &str,
    guild: i64,
    parent: i64,
    original: &ProjectSnapshot,
    matches: impl Fn(&str, &str) -> bool,
) -> Result<Receipt> {
    require(
        !key.trim().is_empty() && guild > 0 && parent > 0 && original.iter().all(|(_, id)| *id > 0),
        "invalid project creation scope",
    )?;
    let receipt = Receipt {
        kind: "project".into(),
        key: key.into(),
        token: uuid::Uuid::new_v4().to_string(),
        guild,
        parent: Some(parent),
        expected: serde_json::to_string(original)
            .map_err(|_| invalid("cannot encode project mapping snapshot"))?,
        phase: "attempted".into(),
        channel: None,
    };
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require(
        find(&tx, key, &matches)?.is_none() && snapshot(&tx, key, &matches)? == *original,
        "project creation already claimed or mapping changed",
    )?;
    attempt_in(&tx, &receipt, None)?;
    require(
        snapshot(&tx, key, &matches)? == *original
            && find(&tx, key, &matches)?.as_ref() == Some(&receipt),
        "project mapping or unique creation claim changed during intent write",
    )?;
    tx.commit()?;
    Ok(receipt)
}

/// Metadata is not authority to adopt an unidentified or another claimed created room.
pub fn ensure_adoptable(path: &Path, channel: i64, parent: i64) -> Result<()> {
    let db = open_initialized(path)?;
    for receipt in records(&db)? {
        require(
            !(receipt.kind == "project"
                && (receipt.channel == Some(channel)
                    || (receipt.channel.is_none() && receipt.parent == Some(parent)))),
            "project inventory candidate may belong to unresolved creation",
        )?;
    }
    Ok(())
}

fn check_other_claims(
    db: &Connection,
    key: &str,
    channel: i64,
    created: bool,
    matches: &impl Fn(&str, &str) -> bool,
) -> Result<()> {
    let known = snapshot(db, key, matches)?
        .iter()
        .any(|(_, id)| *id == channel);
    for receipt in records(db)? {
        if receipt.kind == "project" && !matches(&receipt.key, key) {
            require(
                receipt.channel != Some(channel) && (created || known || receipt.channel.is_some()),
                "project mapping would adopt unresolved creation from another scope",
            )?;
        }
    }
    Ok(())
}

fn check_other_mappings(
    db: &Connection,
    key: &str,
    channel: i64,
    matches: &impl Fn(&str, &str) -> bool,
) -> Result<()> {
    let keys = db
        .prepare("SELECT project_key FROM mirror_projects WHERE discord_channel_id=?")?
        .query_map([channel], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    require(
        keys.iter().all(|stored| matches(stored, key)),
        "created room already belongs to another project",
    )
}

pub(crate) fn before_project_in(
    db: &Connection,
    key: &str,
    channel: i64,
    matches: &impl Fn(&str, &str) -> bool,
) -> Result<Option<Receipt>> {
    let original = find(db, key, matches)?;
    if let Some(receipt) = &original {
        require(
            receipt.phase == "confirmed"
                && receipt.channel == Some(channel)
                && snapshot(db, key, matches)? == expected(receipt)?,
            "project creation mapping is not confirmed",
        )?;
        check_cleanup(db, receipt)?;
        check_other_mappings(db, key, channel, matches)?;
    }
    check_other_claims(db, key, channel, original.is_some(), matches)?;
    Ok(original)
}

/// Same IMMEDIATE transaction as aliases + project mapping; pin custody across every trigger.
pub(crate) fn finish_project_in(
    db: &Connection,
    key: &str,
    channel: i64,
    original: Option<Receipt>,
    matches: &impl Fn(&str, &str) -> bool,
) -> Result<()> {
    require(
        find(db, key, matches)? == original,
        "project creation custody changed during mapping",
    )?;
    let Some(receipt) = original else {
        return check_other_claims(db, key, channel, false, matches);
    };
    let final_mapping = vec![(key.to_owned(), channel)];
    require(
        snapshot(db, key, matches)? == final_mapping,
        "project mapping write was not exact",
    )?;
    check_cleanup(db, &receipt)?;
    check_other_mappings(db, key, channel, matches)?;
    let changed = db.execute(
        "DELETE FROM cdr_mirror_container_creations
         WHERE kind='project' AND scope_key=? AND token=? AND phase='confirmed' AND channel_id=?",
        params![receipt.key, receipt.token, channel],
    )?;
    require(
        changed == 1
            && find(db, key, matches)?.is_none()
            && snapshot(db, key, matches)? == final_mapping,
        "project creation completion was not stored",
    )?;
    check_cleanup(db, &receipt)?;
    check_other_mappings(db, key, channel, matches)?;
    check_other_claims(db, key, channel, true, matches)
}
