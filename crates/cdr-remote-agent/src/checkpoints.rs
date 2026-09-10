mod store;
mod transaction;

use base64::DecodeError;
use cdr_remote_protocol::output::CheckpointEntry;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::files::RemoteFileError;

pub use store::{list, restore, show};
pub use transaction::{MutationTracker, mutate};

pub const MAX_CHECKPOINT_BYTES: usize = 10_485_760;
const MAX_RECORD_BYTES: usize = 33_554_432;

#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error(transparent)]
    File(#[from] RemoteFileError),
    #[error("{0}: invalid checkpoint record")]
    Invalid(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Base64(#[from] DecodeError),
}

#[derive(Clone)]
struct BeforeSnapshot {
    path: String,
    existed: bool,
    content: Vec<u8>,
}

struct CheckpointDraft {
    checkpoint_id: String,
    created_at: String,
    reason: String,
    snapshots: Vec<BeforeSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSnapshot {
    path: String,
    before_exists: bool,
    before_base64: String,
    after_exists: bool,
    after_base64: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCheckpoint {
    checkpoint_id: String,
    created_at: String,
    reason: String,
    snapshots: Vec<StoredSnapshot>,
}

impl StoredCheckpoint {
    fn entry(&self) -> CheckpointEntry {
        CheckpointEntry {
            checkpoint_id: self.checkpoint_id.clone(),
            created_at: self.created_at.clone(),
            reason: self.reason.clone(),
        }
    }
}
