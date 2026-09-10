use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextUsage {
    pub last_input_tokens: u64,
    pub peak_input_tokens: u64,
    pub last_total_tokens: Option<u64>,
    pub model_context_window: Option<u64>,
    pub inferred_compactions: u64,
    pub last_compaction: Option<(u64, u64)>,
    pub observed_at: Option<String>,
}

#[derive(Debug, Error, Eq, PartialEq)]
#[error("context observation is malformed: {0}")]
pub struct ContextUsageError(pub &'static str);

pub fn context_usage_from_events<'a>(
    events: impl IntoIterator<Item = &'a Value>,
) -> Result<Option<ContextUsage>, ContextUsageError> {
    let mut state = Accumulator::default();
    for event in events {
        state.push(event)?;
    }
    Ok(state.finish())
}

#[derive(Default)]
pub(crate) struct Accumulator {
    usage: Option<ContextUsage>,
    window: Option<u64>,
    previous_input: Option<u64>,
}

impl Accumulator {
    pub(crate) fn push(&mut self, event: &Value) -> Result<(), ContextUsageError> {
        if event["type"] != "event_msg" {
            return Ok(());
        }
        let payload = &event["payload"];
        match payload["type"].as_str() {
            Some("task_started") => self.update_window(payload)?,
            Some("token_count") => {
                let info = &payload["info"];
                if info.is_null() {
                    return Ok(());
                }
                if !info.is_object() {
                    return Err(ContextUsageError("token count info is not an object"));
                }
                self.update_window(info)?;
                if let Some(last) = info.get("last_token_usage").filter(|v| !v.is_null()) {
                    self.record(last, event)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn update_window(&mut self, object: &Value) -> Result<(), ContextUsageError> {
        if let Some(window) = object.get("model_context_window") {
            let next = number(window)?.filter(|v| *v > 0);
            if self.window != next {
                self.previous_input = None;
            }
            self.window = next;
        }
        Ok(())
    }

    fn record(&mut self, last: &Value, event: &Value) -> Result<(), ContextUsageError> {
        let input = last
            .get("input_tokens")
            .ok_or(ContextUsageError("input tokens missing"))?
            .as_u64()
            .ok_or(ContextUsageError(
                "input tokens must be a nonnegative integer",
            ))?;
        let total = last.get("total_tokens").map(number).transpose()?.flatten();
        let usage = self.usage.get_or_insert(ContextUsage {
            last_input_tokens: 0,
            peak_input_tokens: 0,
            last_total_tokens: None,
            model_context_window: None,
            inferred_compactions: 0,
            last_compaction: None,
            observed_at: None,
        });
        if let Some(previous) = self.previous_input
            && previous >= 50_000
            && input > 0
            && input < previous
            && u128::from(input) * 5 < u128::from(previous) * 4
            && previous - input >= 25_000
        {
            usage.inferred_compactions = usage.inferred_compactions.saturating_add(1);
            usage.last_compaction = Some((previous, input));
        }
        if input > 0 {
            self.previous_input = Some(input);
        }
        usage.last_input_tokens = input;
        usage.peak_input_tokens = usage.peak_input_tokens.max(input);
        usage.last_total_tokens = total;
        // Keep the window that applied to this token observation, not a later task.
        usage.model_context_window = self.window;
        usage.observed_at = event
            .get("timestamp")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(())
    }

    pub(crate) fn finish(self) -> Option<ContextUsage> {
        self.usage
    }
}

fn number(value: &Value) -> Result<Option<u64>, ContextUsageError> {
    if value.is_null() {
        return Ok(None);
    }
    value.as_u64().map(Some).ok_or(ContextUsageError(
        "token count or window must be a nonnegative integer",
    ))
}
