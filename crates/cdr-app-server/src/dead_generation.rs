use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{RequestId, ServerRequestOccurrence};

/// Called synchronously under the replacement lock, before clearing dead work.
/// Implementations must commit their durable fence before returning success and
/// must not call resident APIs (which can acquire the same replacement lock).
pub trait DeadGenerationFence: Send + Sync {
    fn persist(&self, work: &DeadGenerationWork) -> Result<(), crate::AppServerError>;

    fn check_request(
        &self,
        _generation: u64,
        _method: &str,
        _params: &Value,
    ) -> Result<(), crate::AppServerError> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeadActiveTurn {
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeadServerRequest {
    pub id: RequestId,
    pub occurrence: ServerRequestOccurrence,
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeadGenerationWork {
    pub generation: u64,
    pub closed_reason: String,
    pub active_turns: Vec<DeadActiveTurn>,
    pub server_requests: Vec<DeadServerRequest>,
}

impl DeadGenerationWork {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active_turns.is_empty() && self.server_requests.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadGenerationSettleResult {
    Settled,
    AlreadySettled,
    NotEligible,
    SnapshotChanged,
}
