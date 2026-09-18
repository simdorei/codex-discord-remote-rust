//! Original execution provenance and current-turn ownership are different domains.
use super::Question;
use crate::{
    Result,
    queue::{QueueJobState, StoredQueueJob},
};
use rusqlite::Connection;

pub(super) fn running_matches(job: &StoredQueueJob, q: &Question) -> bool {
    // A non-NULL observation belongs to the exact attached turn. A legacy NULL
    // only retains its original generation; it never adopts the current one.
    let generation = job
        .turn_observation_generation
        .unwrap_or(job.app_server_generation);
    job.job_id == q.origin_job_id
        && job.target_thread_id == q.thread_id
        && job.channel_id == q.channel_id
        && job.owner_user_id == Some(q.owner_user_id)
        && job.state == QueueJobState::Running
        && !job.goal_waiting
        && job.turn_id.as_deref() == Some(q.turn_id.as_str())
        && generation == q.generation
}

pub(super) fn running_owned_in(db: &Connection, q: &Question) -> Result<bool> {
    let ids = db
        .prepare(
            "SELECT job_id FROM codex_turn_queue WHERE target_thread_id=? AND state!='pending'",
        )?
        .query_map([&q.thread_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let [id] = ids.as_slice() else {
        return Ok(false);
    };
    if id != &q.origin_job_id {
        return Ok(false);
    }
    Ok(running_matches(&crate::queue::select_job(db, id)?, q))
}
