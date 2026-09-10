//! Copyable text addressing for a single immutable request occurrence.
use crate::component_worker::ComponentWorkerError;
use cdr_app_server::ServerRequest;

const PREFIX: &str = "[codex-reply:";

pub(crate) fn fingerprint(request: &ServerRequest, generation: u64) -> String {
    cdr_discord::components::request_fingerprint(
        generation,
        request.occurrence.as_bytes(),
        super::component_request_id(&request.id),
    )
}

pub(crate) fn example(request: &ServerRequest, generation: u64) -> String {
    format!(
        "For this exact request, copy the prefix and replace <answer>:\n{PREFIX}{}] <answer>",
        fingerprint(request, generation)
    )
}

pub(crate) fn parse(answer: &str) -> Result<Option<(&str, &str)>, ComponentWorkerError> {
    let Some(rest) = answer.trim_start().strip_prefix(PREFIX) else {
        return Ok(None);
    };
    let Some((token, body)) = rest.split_once(']') else {
        return Err(ComponentWorkerError::NoPendingRequest);
    };
    if token.len() != 32
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !body.starts_with(char::is_whitespace)
        || body.trim().is_empty()
    {
        return Err(ComponentWorkerError::NoPendingRequest);
    }
    Ok(Some((token, body.trim())))
}
