use std::time::Duration;

use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct AppRequest {
    pub method: &'static str,
    pub params: Value,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ThreadSettingsUpdate {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub service_tier: ServiceTierUpdate,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ServiceTierUpdate {
    #[default]
    Unchanged,
    Set(String),
    Clear,
}

#[must_use]
pub fn read_thread(thread_id: &str, include_turns: bool) -> AppRequest {
    read_thread_with_timeout(thread_id, include_turns, Duration::from_secs(8))
}

#[must_use]
pub fn read_thread_with_timeout(
    thread_id: &str,
    include_turns: bool,
    timeout: Duration,
) -> AppRequest {
    request_with_timeout(
        "thread/read",
        json!({"threadId": thread_id, "includeTurns": include_turns}),
        timeout,
    )
}

#[must_use]
pub fn get_goal(thread_id: &str) -> AppRequest {
    thread_request("thread/goal/get", thread_id, 8)
}

#[must_use]
pub fn resume_thread(thread_id: &str) -> AppRequest {
    resume_thread_with_timeout(thread_id, Duration::from_secs(10))
}

#[must_use]
pub fn resume_thread_with_timeout(thread_id: &str, timeout: Duration) -> AppRequest {
    request_with_timeout("thread/resume", json!({"threadId": thread_id}), timeout)
}

#[must_use]
pub fn fork_thread_persistent(thread_id: &str, timeout: Duration) -> AppRequest {
    request_with_timeout(
        "thread/fork",
        json!({"threadId": thread_id, "ephemeral": false}),
        timeout,
    )
}

#[must_use]
pub fn start_thread(cwd: Option<&str>) -> AppRequest {
    let params = cwd.map_or_else(|| json!({}), |cwd| json!({"cwd": cwd}));
    request("thread/start", params, 10)
}

#[must_use]
pub fn update_thread_settings(thread_id: &str, settings: &ThreadSettingsUpdate) -> AppRequest {
    let mut params = Map::from_iter([("threadId".into(), Value::String(thread_id.into()))]);
    if let Some(model) = &settings.model {
        params.insert("model".into(), Value::String(model.clone()));
    }
    if let Some(effort) = &settings.effort {
        params.insert("effort".into(), Value::String(effort.clone()));
    }
    match &settings.service_tier {
        ServiceTierUpdate::Unchanged => {}
        ServiceTierUpdate::Set(tier) => {
            params.insert("serviceTier".into(), Value::String(tier.clone()));
        }
        ServiceTierUpdate::Clear => {
            params.insert("serviceTier".into(), Value::Null);
        }
    }
    request("thread/settings/update", Value::Object(params), 10)
}

#[must_use]
pub fn list_models() -> AppRequest {
    request("model/list", json!({}), 8)
}

#[must_use]
pub fn start_turn(thread_id: &str, prompt: &str) -> AppRequest {
    start_turn_with_input(thread_id, &turn_input(prompt))
}

#[must_use]
pub fn start_turn_with_input(thread_id: &str, input: &[Value]) -> AppRequest {
    request(
        "turn/start",
        json!({"threadId": thread_id, "input": input}),
        12,
    )
}

#[must_use]
pub fn steer_turn(thread_id: &str, prompt: &str, expected_turn_id: &str) -> AppRequest {
    request(
        "turn/steer",
        json!({
            "threadId": thread_id,
            "expectedTurnId": expected_turn_id,
            "input": turn_input(prompt),
        }),
        10,
    )
}

#[must_use]
pub fn interrupt_turn(thread_id: &str, turn_id: &str) -> AppRequest {
    request(
        "turn/interrupt",
        json!({"threadId": thread_id, "turnId": turn_id}),
        10,
    )
}

#[must_use]
pub fn archive_thread(thread_id: &str) -> AppRequest {
    thread_request("thread/archive", thread_id, 10)
}

#[must_use]
pub fn clean_background_terminals(thread_id: &str) -> AppRequest {
    thread_request("thread/backgroundTerminals/clean", thread_id, 10)
}

#[must_use]
pub fn unsubscribe_thread(thread_id: &str) -> AppRequest {
    thread_request("thread/unsubscribe", thread_id, 8)
}

#[must_use]
pub fn rate_limits() -> AppRequest {
    request("account/rateLimits/read", json!({}), 15)
}

#[must_use]
pub fn usage() -> AppRequest {
    request("account/usage/read", json!({}), 15)
}

fn thread_request(method: &'static str, thread_id: &str, seconds: u64) -> AppRequest {
    request(method, json!({"threadId": thread_id}), seconds)
}

fn request(method: &'static str, params: Value, seconds: u64) -> AppRequest {
    request_with_timeout(method, params, Duration::from_secs(seconds))
}

fn request_with_timeout(method: &'static str, params: Value, timeout: Duration) -> AppRequest {
    AppRequest {
        method,
        params,
        timeout,
    }
}

fn turn_input(prompt: &str) -> Vec<Value> {
    vec![json!({"type": "text", "text": prompt, "text_elements": []})]
}
