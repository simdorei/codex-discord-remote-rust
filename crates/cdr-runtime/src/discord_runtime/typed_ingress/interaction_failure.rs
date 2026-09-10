use super::DiscordRuntimeError;
use crate::discord_dispatch::DiscordDispatchError;

/// HTTP failure belongs to one interaction, not the lifetime of the lane.
/// State corruption and admission invariants must still reach the supervisor.
pub(super) fn report_event_result(
    lane: &str,
    interaction_id: u64,
    token: &str,
    result: Result<(), DiscordRuntimeError>,
) -> Result<(), DiscordRuntimeError> {
    match result {
        Err(DiscordRuntimeError::Dispatch(
            error @ (DiscordDispatchError::Acknowledge(_) | DiscordDispatchError::Update(_)),
        )) => {
            let public_error = redacted_error(&error, token);
            eprintln!(
                "discord_interaction_failed lane={lane} interaction={interaction_id} error={public_error}"
            );
            Ok(())
        }
        other => other,
    }
}

pub(super) fn redacted_error(error: &DiscordDispatchError, token: &str) -> String {
    let text = error.to_string();
    if token.is_empty() {
        text
    } else {
        text.replace(token, "[REDACTED]")
    }
}
