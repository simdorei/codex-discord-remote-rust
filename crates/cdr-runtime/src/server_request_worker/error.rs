use cdr_app_server::AppServerError;
use cdr_discord::delivery::DeliveryFailure;
use cdr_store::StoreError;
use thiserror::Error;

use crate::server_prompt::ServerPromptError;

use super::recovery::GenerationMismatch;

#[derive(Debug, Error)]
pub(super) enum ServerPromptSendError {
    #[error(transparent)]
    Discord(#[from] crate::completion_worker::CompletionWorkerError),
    #[error(transparent)]
    Authority(#[from] crate::server_prompt_authority::PromptAuthorityError),
    #[error(transparent)]
    Generation(#[from] GenerationMismatch),
}

#[derive(Debug, Error)]
pub(super) enum ServerRequestWorkerError {
    #[error(transparent)]
    AppServer(#[from] AppServerError),
    #[error(transparent)]
    Prompt(#[from] ServerPromptError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Authority(#[from] crate::server_prompt_authority::PromptAuthorityError),
    #[error("Discord server-request delivery failed: {0:?}")]
    Delivery(DeliveryFailure<ServerPromptSendError>),
    #[error("could not encode app-server request id: {0}")]
    RequestId(serde_json::Error),
    #[error("app-server generation changed during {attempts} pending-request snapshots")]
    UnstableSnapshot { attempts: usize },
    #[error(transparent)]
    StaleGeneration(#[from] GenerationMismatch),
}
