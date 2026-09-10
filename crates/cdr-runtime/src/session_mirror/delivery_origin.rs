use super::{MirrorItem, MirrorKind, SessionMirrorError};
use cdr_store::{
    mirror::{has_event, user_origin_marker},
    queue::{QueueJobState, StoredQueueJob},
};
use std::path::Path;

#[cfg(test)]
#[path = "delivery_origin_tests.rs"]
mod tests;

pub(crate) fn discord_origin_user(
    db: &Path,
    thread: &str,
    item: &MirrorItem,
    jobs: &[StoredQueueJob],
) -> Result<bool, SessionMirrorError> {
    if item.kind != MirrorKind::User {
        return Ok(false);
    }
    if jobs.iter().any(|job| {
        job.target_thread_id == thread
            && job.turn_id.is_some()
            && job.prompt.trim() == item.text.trim()
            && (item.turn_id.is_none() || job.turn_id == item.turn_id)
    }) {
        return Ok(true);
    }
    if let Some(turn) = item.turn_id.as_deref() {
        return Ok(has_event(
            db,
            &user_origin_marker(thread, turn, &item.text),
            thread,
        )?);
    }
    Ok(false)
}

pub(crate) fn discord_active_turn(
    thread: &str,
    item: &MirrorItem,
    jobs: &[StoredQueueJob],
) -> bool {
    jobs.iter().any(|job| {
        job.target_thread_id == thread
            && matches!(job.state, QueueJobState::Starting | QueueJobState::Running)
            && item.turn_id.is_some()
            && job.turn_id == item.turn_id
    })
}
