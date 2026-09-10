use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;

pub const DEFAULT_REQUEST_LIFETIME_SECONDS: i64 = 60;
pub const TRANSPORT_GRACE_SECONDS: i64 = 15;
pub const MAX_COMMAND_RUN_SECONDS: i64 = 300;
pub const MAX_TERMINAL_EXEC_SECONDS: i64 = 3_600;
pub const COMMAND_REQUEST_LIFETIME_SECONDS: i64 = MAX_COMMAND_RUN_SECONDS + TRANSPORT_GRACE_SECONDS;
pub const GIT_REQUEST_LIFETIME_SECONDS: i64 = 120 + TRANSPORT_GRACE_SECONDS;
pub const MAX_REQUEST_LIFETIME_SECONDS: i64 = MAX_TERMINAL_EXEC_SECONDS + TRANSPORT_GRACE_SECONDS;
pub const GATEWAY_REQUEST_TIMEOUT_SECONDS: i64 = MAX_REQUEST_LIFETIME_SECONDS + 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DeadlineError {
    #[error("The local project request expired before execution.")]
    Expired,
    #[error("The local project request was cancelled with its bridge connection.")]
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct RequestBudget {
    deadline: Instant,
}

impl RequestBudget {
    #[must_use]
    pub fn from_deadline(deadline_at: DateTime<Utc>) -> Self {
        let remaining = (deadline_at - Utc::now()).to_std().unwrap_or_default();
        Self {
            deadline: Instant::now() + remaining,
        }
    }

    pub fn remaining(&self, cap: Option<Duration>) -> Result<Duration, DeadlineError> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or(DeadlineError::Expired)?;
        Ok(cap.map_or(remaining, |limit| remaining.min(limit)))
    }
}

#[must_use]
pub fn default_request_deadline() -> DateTime<Utc> {
    Utc::now() + TimeDelta::seconds(DEFAULT_REQUEST_LIFETIME_SECONDS)
}
