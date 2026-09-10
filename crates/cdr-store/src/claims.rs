use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::Result;
use crate::schema::open_initialized;

mod creation;
pub use creation::{create_busy_choice, create_busy_choice_on_route};
pub(crate) use creation::{migrate_schema, schema_current, verify_route};

#[derive(Clone, Copy, Debug)]
pub struct NewBusyChoice<'a> {
    pub owner_user_id: i64,
    pub channel_id: i64,
    pub target_thread_id: Option<&'a str>,
    pub prompt: &'a str,
    pub allow_steer: bool,
    pub now: f64,
    pub time_to_live: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BusyChoice {
    pub choice_id: String,
    pub owner_user_id: i64,
    pub channel_id: i64,
    pub target_thread_id: Option<String>,
    pub prompt: String,
    pub allow_steer: bool,
    pub created_at: f64,
    pub expires_at: f64,
}

pub fn claim_component(path: &Path, claim_key: &str, now: f64, time_to_live: f64) -> Result<bool> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "DELETE FROM persistent_component_claims WHERE expires_at <= ?",
        [now],
    )?;
    let claimed = transaction.execute(
        "INSERT OR IGNORE INTO persistent_component_claims \
         (claim_key, created_at, expires_at) VALUES (?, ?, ?)",
        params![claim_key, now, now + time_to_live],
    )? == 1;
    transaction.commit()?;
    Ok(claimed)
}

pub fn release_component_claim(path: &Path, claim_key: &str) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "DELETE FROM persistent_component_claims WHERE claim_key = ?",
        [claim_key],
    )? == 1)
}

pub fn cleanup_component_claims(path: &Path, now: f64) -> Result<usize> {
    Ok(open_initialized(path)?.execute(
        "DELETE FROM persistent_component_claims WHERE expires_at <= ?",
        [now],
    )?)
}

pub fn component_claim_counts(path: &Path, now: f64) -> Result<(i64, i64)> {
    let connection = open_initialized(path)?;
    let active = connection.query_row(
        "SELECT COUNT(*) FROM persistent_component_claims WHERE expires_at > ?",
        [now],
        |row| row.get(0),
    )?;
    let stale = connection.query_row(
        "SELECT COUNT(*) FROM persistent_component_claims WHERE expires_at <= ?",
        [now],
        |row| row.get(0),
    )?;
    Ok((active, stale))
}

pub fn get_busy_choice(path: &Path, choice_id: &str, now: f64) -> Result<Option<BusyChoice>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row = transaction
        .query_row(
            "SELECT owner_user_id, channel_id, target_thread_id, prompt, allow_steer, \
             created_at, expires_at, claimed_at FROM busy_choices WHERE choice_id = ?",
            [choice_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, Option<f64>>(7)?,
                ))
            },
        )
        .optional()?;
    let choice = match row {
        Some((_, _, _, _, _, _, expires_at, _)) if expires_at <= now => {
            transaction.execute("DELETE FROM busy_choices WHERE choice_id = ?", [choice_id])?;
            None
        }
        // Claimed means in flight or outcome unresolved, not safe to delete.
        // A definite pre-dispatch rejection may release this same choice.
        Some((_, _, _, _, _, _, _, Some(_))) | None => None,
        Some((owner, channel, target, prompt, steer, created, expires, _)) => Some(BusyChoice {
            choice_id: choice_id.into(),
            owner_user_id: owner,
            channel_id: channel,
            target_thread_id: target,
            prompt,
            allow_steer: steer != 0,
            created_at: created,
            expires_at: expires,
        }),
    };
    transaction.commit()?;
    Ok(choice)
}

pub fn claim_busy_choice(path: &Path, choice_id: &str, now: f64) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "UPDATE busy_choices SET claimed_at = ? \
         WHERE choice_id = ? AND claimed_at IS NULL AND expires_at > ?",
        params![now, choice_id, now],
    )? == 1)
}

pub fn release_busy_choice_claim(path: &Path, choice_id: &str) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "UPDATE busy_choices SET claimed_at = NULL WHERE choice_id = ? AND claimed_at IS NOT NULL",
        [choice_id],
    )? == 1)
}

pub fn cleanup_busy_choices(path: &Path, now: f64) -> Result<usize> {
    Ok(open_initialized(path)?.execute("DELETE FROM busy_choices WHERE expires_at <= ?", [now])?)
}

pub fn busy_choice_counts(path: &Path, now: f64) -> Result<(i64, i64)> {
    let connection = open_initialized(path)?;
    let active = connection.query_row(
        "SELECT COUNT(*) FROM busy_choices WHERE expires_at > ? AND claimed_at IS NULL",
        [now],
        |row| row.get(0),
    )?;
    let stale = connection.query_row(
        "SELECT COUNT(*) FROM busy_choices WHERE expires_at <= ? OR claimed_at IS NOT NULL",
        [now],
        |row| row.get(0),
    )?;
    Ok((active, stale))
}
