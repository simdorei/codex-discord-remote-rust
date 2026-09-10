//! Rebuild existing request UIs; this never creates or answers a server request.
use crate::{
    server_prompt::{ServerPrompt, ServerPromptError, build_server_prompt},
    server_prompt_authority::{self, PromptAuthorityError},
};
use cdr_app_server::{ResidentAppServer, ServerRequest};
use std::path::Path;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedPrompt {
    pub request: ServerRequest,
    pub generation: u64,
    pub prompt: ServerPrompt,
    /// Explicit diagnostic only; never a usable authorization UI.
    pub unavailable: bool,
}

#[derive(Debug, Error)]
pub enum RedisplayError {
    #[error(transparent)]
    Authority(#[from] PromptAuthorityError),
    #[error(transparent)]
    Prompt(#[from] ServerPromptError),
    #[error(transparent)]
    Server(#[from] cdr_app_server::AppServerError),
    #[error(
        "pending approval/input changed while preparing its display; no new request was created"
    )]
    Changed,
}

pub async fn prepare(
    database: &Path,
    server: &ResidentAppServer,
    thread: &str,
    channel: u64,
    user: u64,
) -> Result<Vec<PreparedPrompt>, RedisplayError> {
    let generation = server.generation();
    require_ready(server, generation).await?;
    let requests = server.pending_server_requests(Some(thread)).await?;
    let mut prompts = Vec::new();
    for request in &requests {
        prompts.push(prepare_one(database, server, request, generation, channel, user).await?);
    }
    require_ready(server, generation).await?;
    if server.generation() != generation
        || server.pending_server_requests(Some(thread)).await? != requests
    {
        return Err(RedisplayError::Changed);
    }
    Ok(prompts)
}

pub(crate) async fn require_ready(
    server: &ResidentAppServer,
    generation: u64,
) -> Result<(), RedisplayError> {
    let state = server.lifecycle_snapshot().await;
    if state.generation != generation
        || !state.healthy
        || state.quarantined
        || state.restart_pending
    {
        return Err(RedisplayError::Changed);
    }
    Ok(())
}

pub(crate) async fn prepare_one(
    database: &Path,
    server: &ResidentAppServer,
    request: &ServerRequest,
    generation: u64,
    channel: u64,
    user: u64,
) -> Result<PreparedPrompt, RedisplayError> {
    require_ready(server, generation).await?;
    let checked = match server_prompt_authority::verify(database, server, request, generation).await
    {
        Ok(authority) => authority.require_actor(channel, user),
        Err(error) => Err(error),
    };
    let display = match checked {
        Ok(()) => build_server_prompt(request, generation).map_err(RedisplayError::from),
        Err(error) => Err(error.into()),
    };
    let (prompt, unavailable) = match display {
        Ok(prompt) => (prompt, false),
        Err(
            error @ (RedisplayError::Authority(PromptAuthorityError::Invalid(_))
            | RedisplayError::Prompt(_)),
        ) => {
            // Never copy question text, command contents or other request fields into diagnostics.
            let reason = match &error {
                RedisplayError::Prompt(ServerPromptError::Unsupported(_)) => {
                    "unsupported app-server request method".to_owned()
                }
                _ => error.to_string(),
            };
            (
                ServerPrompt {
                    thread_id: cdr_app_server::extract_thread_id(&request.params)
                        .unwrap_or_default(),
                    text: format!("Cannot display this pending request: {reason}"),
                    components: Vec::new(),
                },
                true,
            )
        }
        Err(error) => return Err(error),
    };
    require_ready(server, generation).await?;
    Ok(PreparedPrompt {
        request: request.clone(),
        generation,
        prompt,
        unavailable,
    })
}
