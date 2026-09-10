use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadGoalStatus {
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadGoalUpdate {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub status: ThreadGoalStatus,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GoalParseError {
    #[error("thread/goal/get returned an invalid goal payload")]
    InvalidGoal,
    #[error("thread/goal/get returned a goal for a different thread")]
    DifferentThread,
    #[error("thread goal payload returned an invalid goal status")]
    InvalidStatus,
    #[error("thread goal payload returned an unknown goal status: {0}")]
    UnknownStatus(String),
    #[error("thread/goal/updated had no thread id")]
    MissingThreadId,
    #[error("thread/goal/updated carried a goal for a different thread")]
    UpdateDifferentThread,
}

#[must_use]
pub const fn is_terminal_goal_status(status: ThreadGoalStatus) -> bool {
    matches!(
        status,
        ThreadGoalStatus::Blocked | ThreadGoalStatus::Complete
    )
}

pub fn parse_thread_goal_status(
    result: &Value,
    expected_thread_id: &str,
) -> Result<Option<ThreadGoalStatus>, GoalParseError> {
    let Some(goal) = result.get("goal") else {
        return Ok(None);
    };
    if goal.is_null() {
        return Ok(None);
    }
    let goal = goal.as_object().ok_or(GoalParseError::InvalidGoal)?;
    if goal.get("threadId").and_then(Value::as_str) != Some(expected_thread_id) {
        return Err(GoalParseError::DifferentThread);
    }
    parse_status(goal.get("status")).map(Some)
}

pub fn parse_thread_goal_update(params: &Value) -> Result<ThreadGoalUpdate, GoalParseError> {
    let thread_id = trimmed(params.get("threadId"));
    if thread_id.is_empty() {
        return Err(GoalParseError::MissingThreadId);
    }
    let goal = params
        .get("goal")
        .and_then(Value::as_object)
        .ok_or(GoalParseError::InvalidGoal)?;
    if trimmed(goal.get("threadId")) != thread_id {
        return Err(GoalParseError::UpdateDifferentThread);
    }
    let status = parse_status(goal.get("status"))?;
    let turn_id = nonempty(params.get("turnId"));
    Ok(ThreadGoalUpdate {
        thread_id,
        turn_id,
        status,
    })
}

fn parse_status(value: Option<&Value>) -> Result<ThreadGoalStatus, GoalParseError> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or(GoalParseError::InvalidStatus)?;
    match raw {
        "active" => Ok(ThreadGoalStatus::Active),
        "paused" => Ok(ThreadGoalStatus::Paused),
        "blocked" => Ok(ThreadGoalStatus::Blocked),
        "usageLimited" => Ok(ThreadGoalStatus::UsageLimited),
        "budgetLimited" => Ok(ThreadGoalStatus::BudgetLimited),
        "complete" => Ok(ThreadGoalStatus::Complete),
        other => Err(GoalParseError::UnknownStatus(other.into())),
    }
}

fn nonempty(value: Option<&Value>) -> Option<String> {
    let value = trimmed(value);
    (!value.is_empty()).then_some(value)
}

fn trimmed(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}
