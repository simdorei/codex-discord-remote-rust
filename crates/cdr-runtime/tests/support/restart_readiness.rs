use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cdr_app_server::{AppServerClient, AppServerConfig};
use cdr_runtime::restart_readiness::{
    RestartReadinessError, RestartReadinessState, check_restart_readiness,
};
use cdr_store::mapping::upsert_thread;
use serde_json::{Value, json};

const FIXTURE_ENV: &str = "CDR_RESTART_READINESS_FIXTURE";
const SCENARIO_ENV: &str = "CDR_RESTART_READINESS_SCENARIO";
const METHODS_ENV: &str = "CDR_RESTART_READINESS_METHODS";

pub fn seed_target(path: &Path) {
    upsert_thread(path, "bot-thread", "project", "Bot thread", 70, 71, 1.0).unwrap();
}

pub fn fake_config(scenario: &str, methods: &Path) -> AppServerConfig {
    let mut config = AppServerConfig::new(std::env::current_exe().unwrap());
    config.arguments = vec![
        "--exact".into(),
        "support::fake_app_server_fixture".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    config.environment.insert(FIXTURE_ENV.into(), "1".into());
    config
        .environment
        .insert(SCENARIO_ENV.into(), scenario.into());
    config.environment.insert(
        METHODS_ENV.into(),
        methods.as_os_str().to_string_lossy().into_owned(),
    );
    config
}

#[allow(
    dead_code,
    reason = "shared fixture also compiled by maintenance-only contracts"
)]
pub async fn check_scenario(
    db: &Path,
    scenario: &str,
    quiet: Duration,
    request_timeout: Duration,
) -> Result<(RestartReadinessState, Vec<String>), RestartReadinessError> {
    let methods = db.with_extension(format!("{scenario}.methods"));
    let client = AppServerClient::start(fake_config(scenario, &methods))
        .await
        .unwrap();
    let result = check_restart_readiness(db, &client, quiet, request_timeout).await;
    client.close().await.unwrap();
    result.map(|state| (state, read_methods(&methods)))
}

#[allow(
    dead_code,
    reason = "shared fixture also compiled by maintenance-only contracts"
)]
pub fn read_methods(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
#[ignore = "spawned as a JSON-RPC app-server fixture"]
fn fake_app_server_fixture() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let scenario = std::env::var(SCENARIO_ENV).unwrap();
    let methods = PathBuf::from(std::env::var_os(METHODS_ENV).unwrap());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut reads = 0_u32;
    for line in stdin.lock().lines() {
        let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        record_method(&methods, method);
        let Some(id) = message.get("id") else {
            continue;
        };
        if method == "initialize" {
            send(
                &mut stdout,
                &json!({"id": id, "result": {"userAgent": "fixture/1"}}),
            );
            continue;
        }
        if method != "thread/read" {
            send(
                &mut stdout,
                &json!({"id": id, "error": {"code": -32601, "message": "mutation forbidden"}}),
            );
            continue;
        }
        reads += 1;
        let thread_id = message["params"]["threadId"].as_str().unwrap();
        if scenario == "late_managed" {
            let db = PathBuf::from(std::env::var_os("CDR_MAINTENANCE_FIXTURE_DB").unwrap());
            cdr_store::queue::mark_app_server_managed_target(&db, "bot-thread", 1).unwrap();
        }
        match scenario.as_str() {
            "timeout" => {}
            "server_failure" => send(
                &mut stdout,
                &json!({"id": id, "error": {"code": -32000, "message": "fixture failure"}}),
            ),
            "malformed" => send(
                &mut stdout,
                &json!({"id": id, "result": {"thread": {"id": thread_id, "updatedAt": 1}}}),
            ),
            "busy" => send_thread(&mut stdout, id, thread_id, "active", &[], 1),
            "approval" => send_thread(
                &mut stdout,
                id,
                thread_id,
                "active",
                &["waitingOnApproval"],
                1,
            ),
            "input" => send_thread(
                &mut stdout,
                id,
                thread_id,
                "active",
                &["waitingOnUserInput"],
                1,
            ),
            "recent" => send_thread(&mut stdout, id, thread_id, "idle", &[], unix_now()),
            "eventual" if reads == 1 => {
                send_thread(&mut stdout, id, thread_id, "active", &[], 1);
            }
            "eventual" | "idle" | "late_managed" => {
                send_thread(&mut stdout, id, thread_id, "notLoaded", &[], 1);
            }
            other => send(
                &mut stdout,
                &json!({"id": id, "result": {"thread": {
                    "id": thread_id, "updatedAt": 1, "status": {"type": other}
                }}}),
            ),
        }
    }
}

fn send_thread(
    stdout: &mut impl Write,
    id: &Value,
    thread_id: &str,
    status: &str,
    flags: &[&str],
    updated_at: u64,
) {
    let status = if status == "active" {
        json!({"type": status, "activeFlags": flags})
    } else {
        json!({"type": status})
    };
    send(
        stdout,
        &json!({"id": id, "result": {"thread": {
            "id": thread_id, "updatedAt": updated_at, "status": status
        }}}),
    );
}

fn send(stdout: &mut impl Write, value: &Value) {
    serde_json::to_writer(&mut *stdout, value).unwrap();
    writeln!(stdout).unwrap();
    stdout.flush().unwrap();
}

fn record_method(path: &Path, method: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{method}").unwrap();
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
