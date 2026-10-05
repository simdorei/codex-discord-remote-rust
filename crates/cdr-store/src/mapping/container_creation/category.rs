use super::{Receipt, attempt_in, check_cleanup, check_exact, expected, load, require};
use crate::{Result, schema::open_initialized};
use rusqlite::{TransactionBehavior, params};
use std::path::Path;

pub fn category_receipt(path: &Path, guild: i64) -> Result<Option<Receipt>> {
    let db = open_initialized(path)?;
    let tx = db.unchecked_transaction()?;
    let Some(receipt) = load(&tx, "category", &guild.to_string())? else {
        return Ok(None);
    };
    require(
        receipt.guild == guild && receipt.parent.is_none() && expected(&receipt)?.is_empty(),
        "category creation scope changed",
    )?;
    receipt.confirmed_channel()?;
    check_cleanup(&tx, &receipt)?;
    Ok(Some(receipt))
}

/// A previous bound receipt is supplied only after an exact fresh GET returned 404.
pub fn begin_category(path: &Path, guild: i64, previous: Option<&Receipt>) -> Result<Receipt> {
    require(guild > 0, "invalid category creation scope")?;
    let receipt = Receipt {
        kind: "category".into(),
        key: guild.to_string(),
        token: uuid::Uuid::new_v4().to_string(),
        guild,
        parent: None,
        expected: "[]".into(),
        phase: "attempted".into(),
        channel: None,
    };
    if let Some(previous) = previous {
        require(
            previous.kind == receipt.kind
                && previous.key == receipt.key
                && previous.guild == guild
                && previous.parent.is_none()
                && previous.expected == "[]",
            "category replacement scope changed",
        )?;
    }
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    attempt_in(&tx, &receipt, previous)?;
    tx.commit()?;
    Ok(receipt)
}

pub fn bind_category(path: &Path, receipt: &Receipt) -> Result<()> {
    require(receipt.kind == "category", "invalid category binding")?;
    receipt.confirmed_channel()?;
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_exact(&tx, receipt)?;
    check_cleanup(&tx, receipt)?;
    if !receipt.is_bound() {
        let changed = tx.execute(
            "UPDATE cdr_mirror_container_creations SET phase='bound'
             WHERE kind='category' AND scope_key=? AND token=? AND phase='confirmed' AND channel_id=?",
            params![receipt.key,receipt.token,receipt.channel],
        )?;
        let mut bound = receipt.clone();
        bound.phase = "bound".into();
        require(changed == 1, "category binding was not stored")?;
        check_exact(&tx, &bound)?;
        check_cleanup(&tx, &bound)?;
    }
    tx.commit()?;
    Ok(())
}
