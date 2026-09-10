/// Peel known JSON error envelopes while retaining unknown errors verbatim.
pub(crate) fn readable_error(raw: &str) -> String {
    let mut text = raw.trim().to_owned();
    for _ in 0..4 {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            break;
        };
        let message = value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .or_else(|| value.as_str().map(|_| &value))
            .and_then(serde_json::Value::as_str);
        let Some(message) = message.filter(|value| !value.trim().is_empty()) else {
            break;
        };
        if message == text {
            break;
        }
        message.clone_into(&mut text);
    }
    text
}
