#![cfg(windows)]

use std::collections::BTreeMap;
use std::time::Duration;

use cdr_app_server::browser_host_diagnostics::BrowserHostFieldState;
use cdr_app_server::{AppServerClient, AppServerConfig};
use serde_json::{Value, json};

const PIPE: &str = "BROWSER_USE_ACCESS_PIPE_PATH";
const EXECUTOR: &str = "BROWSER_USE_ACCESS_EXECUTOR_ID";
const IDENTITY: &str = "BROWSER_USE_ACCESS_VERIFYING_IDENTITY";
const KEYS: [&str; 3] = [PIPE, EXECUTOR, IDENTITY];

fn binding(label: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (PIPE.into(), format!(r"\\.\pipe\fixture-browser-{label}")),
        (EXECUTOR.into(), format!("fixture-executor-{label}")),
        (IDENTITY.into(), format!("fixture-verifying-key-{label}")),
    ])
}

fn guarded_failure(executor: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(EXECUTOR.into(), executor.into())])
}

fn run_case(
    inherited: BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
    expected: &BTreeMap<String, String>,
) {
    // Environment mutation is isolated in a separate test process. No live
    // desktop credentials, pipe addresses, or user sessions are consulted.
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "environment_driver", "--ignored", "--nocapture"]);
    for name in KEYS {
        command.env_remove(name);
    }
    command.envs(inherited);
    command.env("CODEX_SESSION_ID", "parent-session-must-not-be-reused");
    command.env("CODEX_THREAD_ID", "parent-thread-must-not-be-reused");
    command.env("CDR_BROWSER_HOST_DRIVER", "1");
    command.env(
        "CDR_BROWSER_HOST_OVERRIDES",
        serde_json::to_string(overrides).unwrap(),
    );
    command.env(
        "CDR_BROWSER_HOST_EXPECTED",
        serde_json::to_string(expected).unwrap(),
    );
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "browser host forwarding fixture failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(log.contains("browser_host_observation"));
    assert!(!log.contains("fixture-") && !log.contains("observed-thread"));
}

#[test]
fn complete_parent_host_context_reaches_the_child_unchanged() {
    run_case(binding("parent"), &BTreeMap::new(), &binding("parent"));
}

#[test]
fn explicit_complete_host_replaces_the_parent_host_as_a_unit() {
    run_case(
        binding("parent"),
        &binding("explicit"),
        &binding("explicit"),
    );
}

#[test]
fn explicit_partial_host_cannot_borrow_the_parent_key_or_executor() {
    run_case(
        binding("parent"),
        &BTreeMap::from([(PIPE.into(), r"\\.\pipe\other-host".into())]),
        &guarded_failure(""),
    );
}

#[test]
fn incomplete_inherited_host_keeps_browser_access_checks_enabled() {
    let mut inherited = binding("parent");
    inherited.remove(IDENTITY);
    run_case(
        inherited,
        &BTreeMap::new(),
        &guarded_failure("fixture-executor-parent"),
    );
}

#[test]
fn an_identity_without_other_host_fields_does_not_disable_access_checks() {
    run_case(
        BTreeMap::from([(IDENTITY.into(), "fixture-key-only".into())]),
        &BTreeMap::new(),
        &guarded_failure(""),
    );
}

#[test]
fn empty_executor_does_not_make_a_partial_host_usable() {
    let mut inherited = binding("parent");
    inherited.insert(EXECUTOR.into(), String::new());
    run_case(inherited, &BTreeMap::new(), &guarded_failure(""));
}

#[test]
fn windows_host_variable_names_are_case_insensitive() {
    let overrides = binding("explicit")
        .into_iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect();
    run_case(binding("parent"), &overrides, &binding("explicit"));
}

#[test]
fn conflicting_case_aliases_do_not_choose_an_arbitrary_host() {
    let mut overrides = binding("explicit");
    overrides.insert(IDENTITY.to_ascii_lowercase(), "conflicting-key".into());
    run_case(binding("parent"), &overrides, &guarded_failure(""));
}

#[test]
fn no_host_context_does_not_invent_connection_metadata() {
    run_case(BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new());
}

#[tokio::test]
#[ignore = "spawned only as an isolated environment fixture"]
async fn environment_driver() {
    if std::env::var_os("CDR_BROWSER_HOST_DRIVER").is_none() {
        return;
    }
    let original: Vec<_> = KEYS
        .iter()
        .map(|key| (*key, std::env::var(key).ok()))
        .collect();
    let expected: BTreeMap<String, String> =
        serde_json::from_str(&std::env::var("CDR_BROWSER_HOST_EXPECTED").unwrap()).unwrap();
    let overrides: BTreeMap<String, String> =
        serde_json::from_str(&std::env::var("CDR_BROWSER_HOST_OVERRIDES").unwrap()).unwrap();
    let mut config = AppServerConfig::new(env!("CARGO_BIN_EXE_fake_codex_app_server"));
    config.arguments.clear();
    config.environment = overrides;
    let client = AppServerClient::start(config).await.unwrap();
    let observed = client.diagnostic_snapshot().browser_host_context;
    let launch = observed
        .launch
        .as_ref()
        .expect("observed Windows child environment");
    let state = |key: &str| match expected.get(key) {
        None => BrowserHostFieldState::Absent,
        Some(value) if value.trim().is_empty() => BrowserHostFieldState::Empty,
        Some(_) => BrowserHostFieldState::Nonempty,
    };
    assert_eq!(launch.child_environment.pipe, state(PIPE));
    assert_eq!(launch.child_environment.executor, state(EXECUTOR));
    assert_eq!(launch.child_environment.verifying_identity, state(IDENTITY));
    assert_eq!(
        observed.worker_environment.pipe,
        BrowserHostFieldState::Unknown
    );
    let encoded = serde_json::to_string(&observed).unwrap();
    assert!(!encoded.contains("fixture-") && !encoded.contains("pipe\\\\"));
    for key in KEYS
        .into_iter()
        .chain(["CODEX_SESSION_ID", "CODEX_THREAD_ID"])
    {
        let result = client
            .request("test/env", json!({"name": key}), Duration::from_secs(2))
            .await
            .unwrap();
        let wanted = expected.get(key).map_or(Value::Null, |value| json!(value));
        assert_eq!(result["value"], wanted, "child environment: {key}");
    }
    for (key, value) in original {
        assert_eq!(std::env::var(key).ok(), value, "parent changed: {key}");
    }
    verify_request_and_failure_observations(&client).await;
    client.close().await.unwrap();
}

async fn verify_request_and_failure_observations(client: &AppServerClient) {
    assert!(
        client
            .diagnostic_snapshot()
            .browser_host_context
            .written_thread_requests
            .is_empty()
    );
    client.request("thread/resume", json!({
        "threadId":"observed-thread", "excludeTurns":true,
        "config":{"mcp_servers.cua_repl.env.BROWSER_USE_ACCESS_EXECUTOR_ID":"fixture-thread-executor"}
    }), Duration::from_secs(2)).await.unwrap();
    let observed = client.diagnostic_snapshot().browser_host_context;
    let request = observed
        .written_thread_requests
        .back()
        .expect("written resume observed");
    assert_eq!(request.method, "thread/resume");
    assert_eq!(
        request.cua_repl_env_override.executor,
        BrowserHostFieldState::Nonempty
    );
    assert_eq!(
        request.cua_repl_env_override.pipe,
        BrowserHostFieldState::Absent
    );
    assert_eq!(
        observed.worker_environment.executor,
        BrowserHostFieldState::Unknown
    );
    assert!(
        !serde_json::to_string(&observed)
            .unwrap()
            .contains("fixture-thread-executor")
    );
    let mut notifications = client.subscribe_notifications();
    client
        .request("test/browserHostFailure", json!({}), Duration::from_secs(2))
        .await
        .unwrap();
    let notification = notifications.try_recv().unwrap();
    assert_eq!(notification.method, "item/completed");
    assert_eq!(notification.params["item"]["status"], "failed");
    let observed = client.diagnostic_snapshot().browser_host_context;
    assert_eq!(
        observed.app_server_pid,
        client.lifecycle_snapshot().process_id
    );
    assert_eq!(observed.reported_tool_failures.len(), 1);
    assert_eq!(
        observed.reported_tool_failures[0].reported_stage,
        "host_context_missing"
    );
    assert_eq!(
        observed.worker_environment.pipe,
        BrowserHostFieldState::Unknown
    );
    assert!(
        !serde_json::to_string(&observed)
            .unwrap()
            .contains("fixture-")
    );
    assert_eq!(
        client
            .request(
                "test/echo",
                json!({"ordinary_rpc_unaffected":true}),
                Duration::from_secs(2)
            )
            .await
            .unwrap(),
        json!({"ordinary_rpc_unaffected":true})
    );
    assert!(client.lifecycle_snapshot().healthy);
}
