mod diff;
mod mutation;
mod process;
mod status;

use thiserror::Error;

use crate::commands::ProcessError;
use crate::files::RemoteFileError;

pub use diff::repo_diff;
pub use mutation::{commit, push};
pub use status::repo_status;

#[derive(Debug, Error)]
pub enum GitError {
    #[error(transparent)]
    File(#[from] RemoteFileError),
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("<git>: {0}")]
    Failed(String),
    #[error("<git>: Git command timed out")]
    TimedOut,
    #[error("<git>: Git command was cancelled")]
    Cancelled,
}
