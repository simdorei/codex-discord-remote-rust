//! Deliver each existing approval/input separately, with a durable receipt for every chunk.
use crate::{
    completion_worker::{
        CompletionWorkerError, IdempotentChunk, send_recorded_message_with_components,
    },
    server_prompt_authority,
    server_prompt_redisplay::PreparedPrompt,
};
use cdr_app_server::ResidentAppServer;
use std::path::Path;
use thiserror::Error;
use twilight_model::id::Id;

pub struct PromptDeliveryContext<'a> {
    pub database: &'a Path,
    pub server: &'a ResidentAppServer,
    pub http: &'a twilight_http::Client,
    pub channel_id: u64,
    pub user_id: u64,
    /// Original Discord command message/interaction identity, not a new random retry id.
    pub command_key: &'a str,
}

#[derive(Debug, Error)]
pub enum PromptDeliveryError {
    #[error(transparent)]
    Authority(#[from] server_prompt_authority::PromptAuthorityError),
    #[error(transparent)]
    Delivery(#[from] CompletionWorkerError),
    #[error(transparent)]
    Identity(#[from] serde_json::Error),
    #[error(transparent)]
    Redisplay(Box<crate::server_prompt_redisplay::RedisplayError>),
}

impl From<crate::server_prompt_redisplay::RedisplayError> for PromptDeliveryError {
    fn from(error: crate::server_prompt_redisplay::RedisplayError) -> Self {
        Self::Redisplay(Box::new(error))
    }
}

pub async fn deliver(
    context: &PromptDeliveryContext<'_>,
    prompts: &[PreparedPrompt],
) -> Result<(), PromptDeliveryError> {
    for prepared in prompts {
        let key = serde_json::to_string(&(
            context.command_key,
            prepared.generation,
            &prepared.request.id,
            prepared.request.occurrence,
        ))?;
        let chunks = cdr_discord::text::split_delivery_chunks(&prepared.prompt.text, true);
        for (index, content) in chunks.iter().enumerate() {
            let current = crate::server_prompt_redisplay::prepare_one(
                context.database,
                context.server,
                &prepared.request,
                prepared.generation,
                context.channel_id,
                context.user_id,
            )
            .await?;
            if current != *prepared
                || !context
                    .server
                    .pending_server_requests(None)
                    .await
                    .map_err(crate::server_prompt_redisplay::RedisplayError::from)?
                    .contains(&prepared.request)
            {
                return Err(crate::server_prompt_redisplay::RedisplayError::Changed.into());
            }
            let components = if index + 1 == chunks.len() {
                prepared.prompt.components.as_slice()
            } else {
                &[]
            };
            send_recorded_message_with_components(
                context.database,
                context.http,
                Id::new(context.channel_id),
                &IdempotentChunk {
                    domain: "server-request/redisplay/v1",
                    logical_key: key.clone(),
                    chunk_index: index,
                    content: content.clone(),
                },
                components,
            )
            .await?;
        }
    }
    Ok(())
}
