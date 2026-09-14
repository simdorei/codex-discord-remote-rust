use super::{QuestionBody, invalid, occurrence_id};
use crate::Result;
use std::path::Path;

pub struct NewQuestion<'a> {
    pub runtime_id: &'a str,
    pub generation: i64,
    pub thread_id: &'a str,
    pub turn_id: &'a str,
    pub item_id: &'a str,
    pub body: &'a QuestionBody,
    pub now: f64,
}

pub fn observe(path: &Path, n: &NewQuestion<'_>) -> Result<String> {
    super::record_observation(path, n)?;
    super::reconcile_observations(path, n.runtime_id, n.generation)?;
    occurrence_id(n.thread_id, n.turn_id, n.item_id, n.body.index)
}

pub(super) fn encode(n: &NewQuestion<'_>) -> Result<String> {
    let body = serde_json::to_string(n.body)?;
    if body.len() > 32_768 || n.runtime_id.is_empty() || !n.now.is_finite() {
        return Err(invalid("invalid or oversized async question"));
    }
    Ok(body)
}
