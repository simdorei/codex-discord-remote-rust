//! Redacted observations, never browser authorization or effective-worker claims.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

const KEYS: [&str; 3] = [
    "BROWSER_USE_ACCESS_PIPE_PATH",
    "BROWSER_USE_ACCESS_EXECUTOR_ID",
    "BROWSER_USE_ACCESS_VERIFYING_IDENTITY",
];
const MAX_OBSERVATIONS: usize = 16;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrowserHostFieldState {
    #[default]
    Unknown,
    Absent,
    Empty,
    Nonempty,
    Ambiguous,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BrowserHostFields {
    pub pipe: BrowserHostFieldState,
    pub executor: BrowserHostFieldState,
    pub verifying_identity: BrowserHostFieldState,
}

impl BrowserHostFields {
    #[cfg(any(windows, test))]
    pub(crate) fn from_environment<'a>(
        values: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        let mut fields = [BrowserHostFieldState::Absent; 3];
        for (key, value) in values {
            if let Some(index) = KEYS
                .iter()
                .position(|known| key.eq_ignore_ascii_case(known))
            {
                observe(&mut fields[index], Some(value));
            }
        }
        Self::from_array(fields)
    }

    fn from_array(fields: [BrowserHostFieldState; 3]) -> Self {
        Self {
            pipe: fields[0],
            executor: fields[1],
            verifying_identity: fields[2],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrowserHostLaunchObservation {
    pub source: &'static str,
    pub inherited: BrowserHostFields,
    pub overrides: BrowserHostFields,
    pub child_environment: BrowserHostFields,
    pub forwarding_result: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrowserHostThreadObservation {
    pub observed_at_unix_ms: Option<u128>,
    pub request_id: String,
    pub method: String,
    pub thread_id_present: bool,
    pub config_present: bool,
    pub node_repl_env_override: BrowserHostFields,
    pub cua_repl_env_override: BrowserHostFields,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrowserHostToolFailure {
    pub observed_at_unix_ms: Option<u128>,
    pub provider: &'static str,
    pub thread_id_present: bool,
    pub turn_id_present: bool,
    pub reported_stage: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BrowserHostDiagnosticSnapshot {
    pub app_server_pid: Option<u32>,
    /// The environment actually used for this child launch, not a later shell.
    /// None means this platform's launch environment was not observed.
    pub launch: Option<BrowserHostLaunchObservation>,
    /// Successful writes only; these do not assert that a resume/start succeeded.
    pub written_thread_requests: VecDeque<BrowserHostThreadObservation>,
    pub reported_tool_failures: VecDeque<BrowserHostToolFailure>,
    /// Configured inputs cannot establish the effective worker environment.
    pub worker_environment: BrowserHostFields,
}

impl BrowserHostDiagnosticSnapshot {
    pub(crate) fn record_thread_request(&mut self, id: String, method: &str, params: &Value) {
        if !matches!(method, "thread/start" | "thread/resume" | "thread/fork") {
            return;
        }
        bounded_push(
            &mut self.written_thread_requests,
            BrowserHostThreadObservation {
                observed_at_unix_ms: now_ms(),
                request_id: id,
                method: method.into(),
                thread_id_present: nonempty_string(&params["threadId"]),
                config_present: params.get("config").is_some_and(|value| !value.is_null()),
                node_repl_env_override: config_fields(params.get("config"), "node_repl"),
                cua_repl_env_override: config_fields(params.get("config"), "cua_repl"),
            },
        );
    }

    pub(crate) fn record_notification(&mut self, method: &str, params: &Value) -> bool {
        if method != "item/completed" {
            return false;
        }
        let item = &params["item"];
        if item["type"] != "mcpToolCall" || item["status"] != "failed" || item["tool"] != "js" {
            return false;
        }
        let provider = match item["server"].as_str() {
            Some("node_repl") => "node_repl",
            Some("cua_repl") => "cua_repl",
            _ => return false,
        };
        // Observe a reported tool error, not page content or successful output.
        // Do not retain its raw message, which can contain URLs or credentials.
        let message = item.pointer("/error/message").and_then(Value::as_str);
        let Some(stage) = message.and_then(reported_stage) else {
            return false;
        };
        bounded_push(
            &mut self.reported_tool_failures,
            BrowserHostToolFailure {
                observed_at_unix_ms: now_ms(),
                provider,
                thread_id_present: nonempty_string(&params["threadId"]),
                turn_id_present: nonempty_string(&params["turnId"]),
                reported_stage: stage,
            },
        );
        true
    }
}

fn config_fields(config: Option<&Value>, provider: &str) -> BrowserHostFields {
    let Some(config) = config.filter(|value| !value.is_null()) else {
        return BrowserHostFields::from_array([BrowserHostFieldState::Absent; 3]);
    };
    let mut fields = [BrowserHostFieldState::Absent; 3];
    let path = format!("mcp_servers.{provider}.env");
    if !observe_config(config, "", &path, &mut fields) {
        return BrowserHostFields::default();
    }
    BrowserHostFields::from_array(fields)
}

/// Accept nested/dotted request representations without guessing precedence.
/// A duplicate host field is ambiguous even when both values happen to match.
fn observe_config(
    value: &Value,
    prefix: &str,
    env_path: &str,
    fields: &mut [BrowserHostFieldState; 3],
) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let mut known = true;
    for (key, value) in object {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if let Some(name) = path.strip_prefix(&format!("{env_path}.")) {
            if let Some(index) = KEYS.iter().position(|key| name.eq_ignore_ascii_case(key)) {
                observe(&mut fields[index], value.as_str());
            }
        } else if path == env_path || env_path.starts_with(&format!("{path}.")) {
            known &= observe_config(value, &path, env_path, fields);
        }
    }
    known
}

fn observe(state: &mut BrowserHostFieldState, value: Option<&str>) {
    *state = if *state == BrowserHostFieldState::Absent {
        match value {
            None => BrowserHostFieldState::Unknown,
            Some(value) if value.trim().is_empty() => BrowserHostFieldState::Empty,
            Some(_) => BrowserHostFieldState::Nonempty,
        }
    } else {
        BrowserHostFieldState::Ambiguous
    };
}

fn reported_stage(message: &str) -> Option<&'static str> {
    let first = message.lines().next()?.trim();
    let first = first.strip_prefix("Error: ").unwrap_or(first);
    match first {
        "The browser access host is unavailable" => Some("host_context_missing"),
        "Cannot verify browser access without the current turn" => Some("turn_context_missing"),
        "Cannot authenticate the browser access response" => Some("response_authentication_failed"),
        "Local browser access is unavailable for this chat" => Some("chat_access_denied"),
        "Browser is not available: chrome" => Some("chrome_not_available"),
        _ => None,
    }
}

fn nonempty_string(value: &Value) -> bool {
    value.as_str().is_some_and(|value| !value.trim().is_empty())
}

fn now_ms() -> Option<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|value| value.as_millis())
}

fn bounded_push<T>(items: &mut VecDeque<T>, item: T) {
    if items.len() == MAX_OBSERVATIONS {
        items.pop_front();
    }
    items.push_back(item);
}

#[cfg(test)]
mod tests;
