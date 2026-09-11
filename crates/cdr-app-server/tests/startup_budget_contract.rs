use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use cdr_app_server::{AppServerClient, AppServerConfig, AppServerError};
use serde_json::json;

const CHILD_ENV: &str = "CDR_STARTUP_BUDGET_FIXTURE";

#[test]
#[ignore = "spawned as an isolated initialization fixture"]
fn delayed_initialization_fixture() {
    let Ok(mode) = std::env::var(CHILD_ENV) else {
        return;
    };
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let message: serde_json::Value = serde_json::from_str(&line).unwrap();
        let Some(id) = message.get("id") else {
            continue;
        };
        if message["method"] == "initialize" {
            if mode == "silent" {
                continue;
            }
            std::thread::sleep(Duration::from_secs(12));
        }
        serde_json::to_writer(&mut stdout, &json!({"id": id, "result": {"ok": true}})).unwrap();
        writeln!(stdout).unwrap();
        stdout.flush().unwrap();
    }
    if let Some(path) = std::env::var_os("CDR_STARTUP_BUDGET_CLOSED") {
        std::fs::write(path, b"closed").unwrap();
    }
}

fn config(mode: &str, closed: &std::path::Path) -> AppServerConfig {
    let mut config = AppServerConfig::new(std::env::current_exe().unwrap());
    config.arguments = vec![
        "--exact".into(),
        "delayed_initialization_fixture".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    config.environment.insert(CHILD_ENV.into(), mode.into());
    config.environment.insert(
        "CDR_STARTUP_BUDGET_CLOSED".into(),
        closed.to_string_lossy().into_owned(),
    );
    config
}

#[tokio::test]
async fn initialization_after_twelve_seconds_is_not_rejected_at_eight() {
    let temp = tempfile::tempdir().unwrap();
    let closed = temp.path().join("closed");
    let result = tokio::time::timeout(
        Duration::from_secs(35),
        AppServerClient::start(config("slow", &closed)),
    )
    .await
    .expect("bounded startup");
    let client =
        result.unwrap_or_else(|error| panic!("INIT-1: slow valid handshake rejected: {error}"));
    assert!(client.lifecycle_snapshot().healthy);
    assert_eq!(
        client
            .request("test/echo", json!({}), Duration::from_secs(2))
            .await
            .unwrap(),
        json!({"ok": true})
    );
    client.close().await.unwrap();
    assert_eq!(std::fs::read(closed).unwrap(), b"closed");
}

#[tokio::test]
async fn silent_initialization_remains_bounded_and_closes_its_child() {
    let temp = tempfile::tempdir().unwrap();
    let closed = temp.path().join("closed");
    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(35),
        AppServerClient::start(config("silent", &closed)),
    )
    .await
    .expect("INIT-2: startup must not wait indefinitely");
    let error = match result {
        Ok(client) => {
            client.close().await.unwrap();
            panic!("silent initialize was accepted")
        }
        Err(error) => error,
    };
    assert!(
        matches!(error, AppServerError::Timeout { ref method, timeout_ms: 30_000 } if method == "initialize")
    );
    assert!(start.elapsed() < Duration::from_secs(35));
    assert_eq!(std::fs::read(closed).unwrap(), b"closed");
}
