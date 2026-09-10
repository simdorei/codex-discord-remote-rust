use std::time::Duration;

use serde_json::Value;

use super::{RestartReadinessError, RestartReadinessState};

pub(super) fn classify(
    result: &Value,
    expected_thread_id: &str,
    quiet: Duration,
    now: u64,
) -> Result<RestartReadinessState, RestartReadinessError> {
    let thread = result
        .get("thread")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(expected_thread_id, "missing thread object"))?;
    let thread_id = thread
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(expected_thread_id, "missing thread id"))?;
    if thread_id != expected_thread_id {
        return Err(invalid(expected_thread_id, "thread id mismatch"));
    }
    let updated_at = thread
        .get("updatedAt")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(expected_thread_id, "missing or invalid updatedAt"))?;
    let status = thread
        .get("status")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(expected_thread_id, "missing status object"))?;
    let status_type = status
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(expected_thread_id, "missing status type"))?;

    match status_type {
        "active" => active(expected_thread_id, status),
        "systemError" => Ok(blocked(format!(
            "thread {expected_thread_id} has systemError status"
        ))),
        "idle" | "notLoaded" => {
            let quiet_seconds = quiet.as_secs();
            if updated_at > now || now - updated_at < quiet_seconds {
                return Ok(blocked(format!(
                    "thread {expected_thread_id} is recent: updated_at={updated_at} quiet_seconds={quiet_seconds}"
                )));
            }
            Ok(RestartReadinessState::Ready)
        }
        other => Err(invalid(
            expected_thread_id,
            &format!("unknown status type {other:?}"),
        )),
    }
}

fn active(
    thread_id: &str,
    status: &serde_json::Map<String, Value>,
) -> Result<RestartReadinessState, RestartReadinessError> {
    let flags = status
        .get("activeFlags")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(thread_id, "active status has no activeFlags array"))?;
    let mut names = Vec::with_capacity(flags.len());
    for flag in flags {
        let name = flag
            .as_str()
            .ok_or_else(|| invalid(thread_id, "active flag is not text"))?;
        if !matches!(name, "waitingOnApproval" | "waitingOnUserInput") {
            return Err(invalid(thread_id, &format!("unknown active flag {name:?}")));
        }
        names.push(name);
    }
    let suffix = if names.is_empty() {
        String::new()
    } else {
        format!(" flags={}", names.join(","))
    };
    Ok(blocked(format!("thread {thread_id} is active{suffix}")))
}

fn blocked(reason: String) -> RestartReadinessState {
    RestartReadinessState::Blocked { reason }
}

fn invalid(thread_id: &str, reason: &str) -> RestartReadinessError {
    RestartReadinessError::InvalidThreadState {
        thread_id: thread_id.to_owned(),
        reason: reason.to_owned(),
    }
}
