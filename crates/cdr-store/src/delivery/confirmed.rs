//! Retire already-delivered finals without reopening the same catalog per check.
use std::path::Path;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use super::{FinalReadiness, StoredDelivery, preflight, select};
use crate::{Result, delivery_receipt, schema};

/// The trusted runtime supplies every normally rendered and validated chunk.
/// Missing, changed or unconfirmed receipts roll back all tentative claims;
/// the ordinary delivery path remains responsible for sending and errors.
pub fn complete_confirmed(
    path: &Path,
    pending: &StoredDelivery,
    chunks: &[String],
) -> Result<bool> {
    if chunks.is_empty() || pending.channel_id <= 0 {
        return Ok(false);
    }
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(schema::STORE_BUSY_TIMEOUT)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    schema::checked_read::verify_current_catalog_in(&tx)?;
    if select(&tx, &pending.delivery_id)? != *pending
        || preflight::read(&tx, pending)? != FinalReadiness::Ready
    {
        return Ok(false);
    }
    let guard = crate::new_reply::DeliveryGuard {
        job_id: &pending.job_id,
        thread_id: &pending.target_thread_id,
        turn_id: &pending.turn_id,
    };
    for (index, chunk) in chunks.iter().enumerate() {
        let key = serde_json::to_string(&(
            pending.channel_id,
            "completion/v1",
            &pending.delivery_id,
            index,
        ))?;
        let hash = crate::final_recovery::sha256(chunk);
        if !matches!(
            delivery_receipt::begin_guarded_in(&tx, &key, &hash, Some(&guard))?,
            delivery_receipt::ReceiptState::Delivered(_)
        ) {
            return Ok(false);
        }
    }
    if tx.execute(
        "DELETE FROM codex_delivery_outbox WHERE delivery_id=?",
        [&pending.delivery_id],
    )? != 1
    {
        return Ok(false);
    }
    tx.commit()?;
    Ok(true)
}
