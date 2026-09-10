use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

pub type BoxPromptFuture<'a> =
    Pin<Box<dyn Future<Output = Result<String, PromptPreprocessError>> + Send + 'a>>;

pub trait PromptPreprocessor: Send + Sync {
    fn prepare<'a>(&'a self, prompt: &'a str, thread_id: &'a str) -> BoxPromptFuture<'a>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{public_message} {recovery_action}")]
pub struct PromptPreprocessError {
    pub public_message: String,
    pub recovery_action: String,
}
