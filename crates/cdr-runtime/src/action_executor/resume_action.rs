use super::{ActionError, ActionExecutor, ActionResult, immediate};
use crate::queue_runner::TurnBackend;
use cdr_app_server::{
    ResidentAppServer,
    requests::{read_thread_with_timeout, resume_thread_with_timeout},
};
use serde_json::Value;
use std::time::Duration;
use tokio::time::Instant;

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn resume_thread(
        &self,
        channel: u64,
        reference: Option<&str>,
    ) -> Result<ActionResult, ActionError> {
        self.resume_bound(channel, reference, None).await
    }

    pub(super) async fn resume_bound(
        &self,
        channel: u64,
        reference: Option<&str>,
        binding: Option<&crate::settings_binding::SettingsBinding>,
    ) -> Result<ActionResult, ActionError> {
        let thread =
            self.resolve_thread(channel, binding.map(|b| b.target.as_str()).or(reference))?;
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let deadline = Instant::now() + self.app_server_resume_timeout;
        let _guard = tokio::time::timeout_at(deadline, self.control_lock(&thread.id))
            .await
            .map_err(|_| {
                ActionError::Invalid("resume control wait timed out; no prompt was resent".into())
            })??;
        let generation = server.generation();
        self.validate_lifecycle_binding(binding, channel)?;
        if reference.is_none() && self.target(channel)?.0 != thread.id {
            return Err(ActionError::Invalid(
                "resume target changed; no request was sent".into(),
            ));
        }
        healthy_generation(server, generation).await?;
        let outcome = tokio::time::timeout_at(deadline, recover(server, &thread.id, generation, deadline)).await
            .map_err(|_| ActionError::Invalid("resume verification timed out; loaded state is unverified".into()))
            .and_then(|result| result)
            .map_err(|error| ActionError::Invalid(format!("resume verification failed for {}: {error}. No prompt was resent; no fork was used.", thread.id)))?;
        healthy_generation(server, generation).await?;
        if reference.is_none() && self.target(channel)?.0 != thread.id {
            return Err(ActionError::Invalid("resume target changed during verification; original thread retained, no prompt was resent".into()));
        }
        self.validate_lifecycle_binding(binding, channel)?;
        self.bridge_state.set_selected_thread_id(Some(&thread.id))?;
        Ok(immediate(format!(
            "Codex thread resume check complete.\nthread_id: {}\nstatus: {outcome}\nNo prompt was resent.",
            thread.id
        )))
    }
}

async fn healthy_generation(
    server: &ResidentAppServer,
    generation: u64,
) -> Result<(), ActionError> {
    let state = server.lifecycle_snapshot().await;
    if !state.healthy
        || state.quarantined
        || state.restart_pending
        || server.generation() != generation
    {
        return Err(ActionError::Invalid("resume connection is unavailable or changed; no verified success, no prompt was resent".into()));
    }
    Ok(())
}

async fn recover(
    server: &ResidentAppServer,
    thread: &str,
    generation: u64,
    deadline: Instant,
) -> Result<&'static str, ActionError> {
    if read_status(server, thread, generation, deadline).await? {
        return Ok("already loaded");
    }
    let response = server
        .execute(
            resume_thread_with_timeout(thread, remaining(deadline)?),
            Some(generation),
        )
        .await?;
    if response.pointer("/thread/id").and_then(Value::as_str) != Some(thread) {
        return Err(ActionError::Invalid(
            "thread/resume returned a missing or different thread identity".into(),
        ));
    }
    if !read_status(server, thread, generation, deadline).await? {
        return Err(ActionError::Invalid(
            "thread is still not loaded after resume".into(),
        ));
    }
    Ok("recovered")
}

async fn read_status(
    server: &ResidentAppServer,
    thread: &str,
    generation: u64,
    deadline: Instant,
) -> Result<bool, ActionError> {
    let response = server
        .execute(
            read_thread_with_timeout(
                thread,
                false,
                remaining(deadline)?.min(Duration::from_secs(8)),
            ),
            Some(generation),
        )
        .await?;
    if response.pointer("/thread/id").and_then(Value::as_str) != Some(thread) {
        return Err(ActionError::Invalid(
            "thread/read returned a missing or different thread identity".into(),
        ));
    }
    match response
        .pointer("/thread/status/type")
        .and_then(Value::as_str)
    {
        Some("idle" | "active") => Ok(true),
        Some("notLoaded") => Ok(false),
        Some("systemError") => Err(ActionError::Invalid("thread reports systemError".into())),
        _ => Err(ActionError::Invalid(
            "thread runtime status is missing or unsupported".into(),
        )),
    }
}

fn remaining(deadline: Instant) -> Result<Duration, ActionError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| ActionError::Invalid("resume check timed out".into()))
}
