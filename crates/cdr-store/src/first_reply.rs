//! First visible reply barrier for a durably owned Discord request.
use crate::{Result, schema::open_initialized};
use rusqlite::OptionalExtension;
use std::path::Path;

pub fn pending(path: &Path, job_id: &str) -> Result<Option<String>> {
    // A headless Action has no Discord first reply. Canonical duplicate clicks
    // must not close the barrier again after the original request was confirmed.
    let row = open_initialized(path)?
        .query_row(
            "SELECT ingress_id,confirmation_delivered FROM discord_ingress_journal
         WHERE owner_kind='prompt' AND owner_id=? AND kind IN ('message','interaction')
         ORDER BY created_at,ingress_id LIMIT 1",
            [job_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(id, confirmed)| (!confirmed).then_some(id)))
}
