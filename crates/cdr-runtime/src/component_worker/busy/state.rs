use std::path::Path;

use cdr_store::Result;
use cdr_store::claims::BusyChoice;
use cdr_store::schema::open_initialized;
use rusqlite::OptionalExtension;

#[derive(Clone, Debug, PartialEq)]
pub struct BusyChoiceState {
    pub choice: BusyChoice,
    pub claimed: bool,
}

pub fn read_busy_choice_state(
    database: &Path,
    choice_id: &str,
    now: f64,
) -> Result<Option<BusyChoiceState>> {
    let row = open_initialized(database)?
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
    let Some((owner, channel, target, prompt, steer, created, expires, claimed_at)) = row else {
        return Ok(None);
    };
    if expires <= now {
        return Ok(None);
    }
    Ok(Some(BusyChoiceState {
        choice: BusyChoice {
            choice_id: choice_id.into(),
            owner_user_id: owner,
            channel_id: channel,
            target_thread_id: target,
            prompt,
            allow_steer: steer != 0,
            created_at: created,
            expires_at: expires,
        },
        claimed: claimed_at.is_some(),
    }))
}
