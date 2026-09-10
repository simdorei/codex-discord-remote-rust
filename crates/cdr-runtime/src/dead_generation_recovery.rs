//! Durable fencing installed before sharing the resident app-server.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cdr_app_server::{AppServerError, DeadGenerationFence, DeadGenerationWork, extract_thread_id};
use cdr_store::dead_generation::{
    DeadGenerationCapture, activate_runtime, capture_dead_generation, generation_is_sealed,
    target_is_held,
};
use serde_json::Value;

pub struct RuntimeDeadGenerationFence {
    mirror_db: PathBuf,
    runtime_id: String,
    startup_channel_id: Option<i64>,
}

impl RuntimeDeadGenerationFence {
    /// Call only after acquiring the bot's single-instance guard, before any
    /// app-server clients or queue workers can run.
    pub fn new(
        mirror_db: PathBuf,
        runtime_id: String,
        startup_channel_id: Option<u64>,
    ) -> Result<Self, AppServerError> {
        let startup_channel_id = startup_channel_id
            .map(i64::try_from)
            .transpose()
            .map_err(failure)?;
        activate_runtime(&mirror_db, &runtime_id).map_err(failure)?;
        Ok(Self {
            mirror_db,
            runtime_id,
            startup_channel_id,
        })
    }
}

impl DeadGenerationFence for RuntimeDeadGenerationFence {
    fn persist(&self, work: &DeadGenerationWork) -> Result<(), AppServerError> {
        let affected_targets = work
            .active_turns
            .iter()
            .map(|turn| turn.thread_id.clone())
            .chain(
                work.server_requests
                    .iter()
                    .filter_map(|request| extract_thread_id(&request.params)),
            )
            .collect::<Vec<_>>();
        let has_unscoped_requests = work
            .server_requests
            .iter()
            .any(|request| extract_thread_id(&request.params).is_none());
        let snapshot_json = serde_json::to_string(work)?;
        capture_dead_generation(
            &self.mirror_db,
            DeadGenerationCapture {
                runtime_id: &self.runtime_id,
                generation: i64::try_from(work.generation).map_err(failure)?,
                snapshot_json: &snapshot_json,
                affected_targets: &affected_targets,
                startup_channel_id: self.startup_channel_id,
                has_unscoped_requests,
                now: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(failure)?
                    .as_secs_f64(),
            },
        )
        .map_err(failure)?;
        Ok(())
    }

    fn check_request(
        &self,
        generation: u64,
        method: &str,
        params: &Value,
    ) -> Result<(), AppServerError> {
        if !matches!(
            method,
            "thread/resume" | "thread/fork" | "turn/start" | "turn/steer"
        ) {
            return Ok(());
        }
        if let Some(target) = extract_thread_id(params)
            && target_is_held(&self.mirror_db, &target).map_err(failure)?
        {
            return Err(failure(cdr_store::StoreError::DeadGenerationTargetHeld(
                target,
            )));
        }
        if generation_is_sealed(&self.mirror_db, i64::try_from(generation).map_err(failure)?)
            .map_err(failure)?
        {
            return Err(failure("app-server generation is durably sealed"));
        }
        Ok(())
    }
}

fn failure(error: impl std::fmt::Display) -> AppServerError {
    AppServerError::DeadGenerationFence {
        message: error.to_string(),
    }
}
