use super::{FailurePhase, UNRESOLVED_FORK_ERROR_PREFIX};

const LAST_ERROR_LIMIT: usize = 1_000;
const PREVIOUS_ERROR_LIMIT: usize = 240;
const PREVIOUS_ERROR_LABEL: &str = "\nPrevious error: ";

pub(super) fn prior_error(stored_error: &str, previous_fork_error: &str) -> String {
    let stored_error = stored_error.trim();
    let Some(encoded) = stored_error.strip_prefix(UNRESOLVED_FORK_ERROR_PREFIX) else {
        return bounded(stored_error, PREVIOUS_ERROR_LIMIT);
    };
    if stored_error == unresolved_message(previous_fork_error, "") {
        return String::new();
    }
    let candidate = encoded
        .rsplit_once(PREVIOUS_ERROR_LABEL)
        .map(|(_, previous)| previous)
        .unwrap_or_default();
    if candidate.starts_with(UNRESOLVED_FORK_ERROR_PREFIX)
        || stored_error != unresolved_message(previous_fork_error, candidate)
    {
        String::new()
    } else {
        bounded(candidate, PREVIOUS_ERROR_LIMIT)
    }
}

pub(super) fn unresolved_message(fork_error: &str, previous_error: &str) -> String {
    let fork_error = fork_error
        .trim()
        .trim_start_matches(UNRESOLVED_FORK_ERROR_PREFIX)
        .trim_start();
    let previous = bounded(previous_error, PREVIOUS_ERROR_LIMIT);
    let previous_overhead = if previous.is_empty() {
        0
    } else {
        PREVIOUS_ERROR_LABEL.chars().count() + previous.chars().count()
    };
    let overhead = UNRESOLVED_FORK_ERROR_PREFIX.chars().count() + previous_overhead;
    let latest = bounded(fork_error, LAST_ERROR_LIMIT.saturating_sub(overhead));
    if previous.is_empty() {
        format!("{UNRESOLVED_FORK_ERROR_PREFIX}{latest}")
    } else {
        format!("{UNRESOLVED_FORK_ERROR_PREFIX}{latest}{PREVIOUS_ERROR_LABEL}{previous}")
    }
}

pub(super) fn unresolved_notice(
    fork_error: &str,
    previous_error: &str,
    phase: FailurePhase,
) -> String {
    let mut content = match phase {
        FailurePhase::ForkOutcome => format!(
            "The Codex ownership fork result is uncertain. This request remains queued and will not run until recovery, preventing a duplicate response.\nFork error: {fork_error}"
        ),
        FailurePhase::Finalize => format!(
            "The Codex ownership fork target was created, but local routing finalization failed. This request remains queued and will not run until recovery, preventing a duplicate response.\nFinalize error: {fork_error}"
        ),
        FailurePhase::Cancellation => format!(
            "The Codex ownership fork failed and its cancellation could not be confirmed. This request remains queued and will not run until recovery, preventing a duplicate response.\nFailure details: {fork_error}"
        ),
    };
    if !previous_error.is_empty() {
        content.push_str(PREVIOUS_ERROR_LABEL);
        content.push_str(previous_error);
    }
    content
}

fn bounded(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}
