//! Approval authority comes from the exact original running request, never a new click.
use cdr_app_server::{ResidentAppServer, ServerRequest, extract_thread_id};
use cdr_store::queue::{self, QueueJobState};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PromptAuthorityError {
    #[error("approval/input request authority is unavailable: {0}; no response was submitted")]
    Invalid(&'static str),
    #[error(transparent)]
    Store(#[from] cdr_store::StoreError),
    #[error(transparent)]
    Server(#[from] cdr_app_server::AppServerError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptAuthority {
    pub thread_id: String,
    pub turn_id: String,
    pub channel_id: u64,
    pub user_id: u64,
    pub generation: u64,
}

impl PromptAuthority {
    pub fn require_actor(&self, channel: u64, user: u64) -> Result<(), PromptAuthorityError> {
        if self.channel_id != channel || self.user_id != user {
            return Err(PromptAuthorityError::Invalid(
                "original user or channel does not match",
            ));
        }
        Ok(())
    }
}

pub async fn verify(
    db: &Path,
    server: &ResidentAppServer,
    request: &ServerRequest,
    generation: u64,
) -> Result<PromptAuthority, PromptAuthorityError> {
    reject_secret_input(request)?;
    let thread = extract_thread_id(&request.params)
        .ok_or(PromptAuthorityError::Invalid("missing original thread"))?;
    // A top-level item/request id is not evidence of a turn id.
    let turn = request
        .params
        .get("turnId")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty() && id.trim() == *id)
        .ok_or(PromptAuthorityError::Invalid("missing original turn"))?;
    let snapshot = server.lifecycle_snapshot().await;
    if snapshot.generation != generation
        || !snapshot.healthy
        || snapshot.quarantined
        || snapshot.restart_pending
    {
        return Err(PromptAuthorityError::Invalid(
            "connection changed or is not ready",
        ));
    }
    if server.active_turn_id(&thread).await?.as_deref() != Some(turn)
        || cdr_store::observed_completion::contains(db, &thread, turn)?
    {
        return Err(PromptAuthorityError::Invalid(
            "original turn is no longer active",
        ));
    }
    let jobs = queue::list(db)?;
    let mut matches = jobs
        .iter()
        .filter(|job| job.target_thread_id == thread && job.turn_id.as_deref() == Some(turn));
    let job = matches.next().ok_or(PromptAuthorityError::Invalid(
        "original Discord request owner is unknown",
    ))?;
    if matches.next().is_some()
        || job.state != QueueJobState::Running
        || job.goal_waiting
        || u64::try_from(job.app_server_generation).ok() != Some(generation)
    {
        return Err(PromptAuthorityError::Invalid(
            "original execution ownership is uncertain",
        ));
    }
    let channel = u64::try_from(job.channel_id)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(PromptAuthorityError::Invalid("invalid original channel"))?;
    let user = job
        .owner_user_id
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(PromptAuthorityError::Invalid(
            "original Discord user is unknown",
        ))?;
    if cdr_store::mapping::mirrored_thread_id(db, Some(job.channel_id))?
        .is_some_and(|mapped| mapped != thread)
    {
        return Err(PromptAuthorityError::Invalid(
            "original channel mapping changed",
        ));
    }
    if server.generation() != generation
        || !server
            .pending_server_requests(Some(&thread))
            .await?
            .iter()
            .any(|pending| pending == request)
    {
        return Err(PromptAuthorityError::Invalid(
            "original request expired or changed",
        ));
    }
    Ok(PromptAuthority {
        thread_id: thread,
        turn_id: turn.into(),
        channel_id: channel,
        user_id: user,
        generation,
    })
}

fn reject_secret_input(request: &ServerRequest) -> Result<(), PromptAuthorityError> {
    if request.method == "item/tool/requestUserInput"
        && request
            .params
            .get("questions")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|questions| {
                questions
                    .iter()
                    .any(|q| q.get("isSecret").and_then(serde_json::Value::as_bool) == Some(true))
            })
    {
        return Err(PromptAuthorityError::Invalid(
            "secret input requires the Codex app",
        ));
    }
    Ok(())
}
