use super::*;
use serde_json::json;

#[test]
fn environment_states_are_redacted_and_case_collisions_are_ambiguous() {
    let fields = BrowserHostFields::from_environment([
        (KEYS[0], "private-pipe"),
        ("browser_use_access_pipe_path", "different-private-pipe"),
        (KEYS[1], "  "),
        (KEYS[2], "private-key"),
    ]);
    assert_eq!(fields.pipe, BrowserHostFieldState::Ambiguous);
    assert_eq!(fields.executor, BrowserHostFieldState::Empty);
    assert_eq!(fields.verifying_identity, BrowserHostFieldState::Nonempty);
    let report = serde_json::to_string(&fields).unwrap();
    assert!(!report.contains("private"));
}

#[test]
fn absent_thread_overrides_do_not_establish_an_absent_worker_context() {
    let mut report = BrowserHostDiagnosticSnapshot::default();
    report.record_thread_request(
        "request-a".into(),
        "thread/start",
        &json!({"cwd":"private-cwd"}),
    );
    let request = &report.written_thread_requests[0];
    assert!(!request.config_present);
    assert_eq!(
        request.cua_repl_env_override.pipe,
        BrowserHostFieldState::Absent
    );
    assert_eq!(
        report.worker_environment.pipe,
        BrowserHostFieldState::Unknown
    );
    assert!(report.launch.is_none());
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("private-cwd")
    );
}

#[test]
fn dotted_and_nested_config_are_observed_without_mutating_the_request() {
    let params = json!({
        "threadId":"private-thread",
        "excludeTurns":true,
        "config":{
            "mcp_servers.cua_repl.env":{
                "BROWSER_USE_ACCESS_PIPE_PATH":"private-pipe",
                "BROWSER_USE_ACCESS_EXECUTOR_ID":"private-executor"
            },
            "mcp_servers.cua_repl.env.BROWSER_USE_ACCESS_VERIFYING_IDENTITY":"private-key",
            "mcp_servers":{"node_repl":{"env":{"BROWSER_USE_ACCESS_EXECUTOR_ID":""}}},
            "model":"preserved-model"
        }
    });
    let original = params.clone();
    let mut report = BrowserHostDiagnosticSnapshot::default();
    report.record_thread_request("request-b".into(), "thread/resume", &params);
    assert_eq!(params, original);
    let request = &report.written_thread_requests[0];
    assert!(request.config_present && request.thread_id_present);
    assert_eq!(
        request.cua_repl_env_override.pipe,
        BrowserHostFieldState::Nonempty
    );
    assert_eq!(
        request.cua_repl_env_override.executor,
        BrowserHostFieldState::Nonempty
    );
    assert_eq!(
        request.cua_repl_env_override.verifying_identity,
        BrowserHostFieldState::Nonempty
    );
    assert_eq!(
        request.node_repl_env_override.executor,
        BrowserHostFieldState::Empty
    );
    assert_eq!(
        request.node_repl_env_override.pipe,
        BrowserHostFieldState::Absent
    );
    assert_eq!(report.worker_environment, BrowserHostFields::default());
    let encoded = serde_json::to_string(&report).unwrap();
    assert!(!encoded.contains("private-") && !encoded.contains("preserved-model"));
}

#[test]
fn mixed_config_aliases_never_guess_effective_precedence() {
    let fields = config_fields(
        Some(&json!({
            "mcp_servers.cua_repl.env.BROWSER_USE_ACCESS_PIPE_PATH":"one",
            "mcp_servers":{"cua_repl":{"env":{"browser_use_access_pipe_path":"two"}}}
        })),
        "cua_repl",
    );
    assert_eq!(fields.pipe, BrowserHostFieldState::Ambiguous);
    assert_eq!(fields.executor, BrowserHostFieldState::Absent);
}

#[test]
fn malformed_config_is_unknown_not_missing() {
    for config in [json!(false), json!({"mcp_servers.cua_repl.env":false})] {
        assert_eq!(
            config_fields(Some(&config), "cua_repl"),
            BrowserHostFields::default()
        );
    }
    let fields = config_fields(
        Some(&json!({
            "mcp_servers.cua_repl.env.BROWSER_USE_ACCESS_PIPE_PATH":42
        })),
        "cua_repl",
    );
    assert_eq!(fields.pipe, BrowserHostFieldState::Unknown);
}

#[test]
fn observations_are_bounded_and_ordinary_requests_are_not_recorded() {
    let mut report = BrowserHostDiagnosticSnapshot::default();
    for index in 0..40 {
        report.record_thread_request(index.to_string(), "thread/resume", &json!({}));
        report.record_thread_request("not-recorded".into(), "turn/start", &json!({}));
        report.record_thread_request("not-recorded".into(), "mcpServerStatus/list", &json!({}));
    }
    assert_eq!(report.written_thread_requests.len(), MAX_OBSERVATIONS);
    assert_eq!(report.written_thread_requests[0].request_id, "24");
    assert_eq!(report.worker_environment, BrowserHostFields::default());
}

fn failure() -> Value {
    json!({
        "threadId":"private-thread", "turnId":"private-turn",
        "item":{
            "type":"mcpToolCall", "server":"cua_repl", "tool":"js", "status":"failed",
            "error":{"message":"Error: The browser access host is unavailable\n    at private-pipe-and-key"}
        }
    })
}

#[test]
fn only_failed_tool_error_metadata_is_classified_and_never_retained() {
    let mut report = BrowserHostDiagnosticSnapshot::default();
    report.record_notification("item/completed", &failure());
    let event = &report.reported_tool_failures[0];
    assert_eq!(event.reported_stage, "host_context_missing");
    assert!(event.thread_id_present && event.turn_id_present);
    assert!(!serde_json::to_string(&report).unwrap().contains("private-"));
    assert_eq!(report.worker_environment, BrowserHostFields::default());
}

#[test]
fn successful_output_and_other_providers_cannot_claim_browser_access_status() {
    for (path, value) in [
        ("/item/status", json!("completed")),
        ("/item/type", json!("agentMessage")),
        ("/item/server", json!("unrelated")),
        ("/item/tool", json!("other")),
        (
            "/item/error/message",
            json!("page text: The browser access host is unavailable"),
        ),
    ] {
        let mut event = failure();
        *event.pointer_mut(path).unwrap() = value;
        let mut report = BrowserHostDiagnosticSnapshot::default();
        report.record_notification("item/completed", &event);
        assert!(report.reported_tool_failures.is_empty(), "{path}");
    }
    let mut report = BrowserHostDiagnosticSnapshot::default();
    report.record_notification("item/started", &failure());
    assert!(report.reported_tool_failures.is_empty());
}

#[test]
fn reported_stages_stay_distinct_and_failure_history_is_bounded() {
    for (message, stage) in [
        (
            "Cannot verify browser access without the current turn",
            "turn_context_missing",
        ),
        (
            "Cannot authenticate the browser access response",
            "response_authentication_failed",
        ),
        (
            "Local browser access is unavailable for this chat",
            "chat_access_denied",
        ),
        ("Browser is not available: chrome", "chrome_not_available"),
    ] {
        assert_eq!(reported_stage(message), Some(stage));
    }
    let mut report = BrowserHostDiagnosticSnapshot::default();
    for _ in 0..50 {
        report.record_notification("item/completed", &failure());
    }
    assert_eq!(report.reported_tool_failures.len(), MAX_OBSERVATIONS);
}
