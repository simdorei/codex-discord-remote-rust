use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Duration;

use cdr_app_server::{AppServerConfig, ResidentAppServer};
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

const FIXTURE_ENV: &str = "CDR_NEW_THREAD_FIXTURE";
const METHODS_ENV: &str = "CDR_NEW_THREAD_METHODS";
pub const TEST_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn start_server(log: &Path, scenario: &str) -> ResidentAppServer {
    let mut config = AppServerConfig::new(std::env::current_exe().unwrap());
    config.arguments = vec![
        "--exact".into(),
        "support::fake_app_server_fixture".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    config
        .environment
        .insert(FIXTURE_ENV.into(), scenario.into());
    config
        .environment
        .insert(METHODS_ENV.into(), log.to_string_lossy().into_owned());
    ResidentAppServer::start(config).await.unwrap()
}

pub fn starts(log: &Path) -> usize {
    fs::read_to_string(log)
        .unwrap()
        .lines()
        .filter(|line| *line == "thread/start")
        .count()
}

pub async fn wait_for_start(log: &Path) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        while starts(log) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fixture never received thread/start");
}

pub struct PreparationGate {
    pub entered: Semaphore,
}

impl PreparationGate {
    pub fn new() -> Self {
        Self {
            entered: Semaphore::new(0),
        }
    }
}

impl PromptPreprocessor for PreparationGate {
    fn prepare<'a>(&'a self, _prompt: &'a str, _thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            self.entered.add_permits(1);
            std::future::pending().await
        })
    }
}

#[test]
#[ignore = "spawned as a Rust JSON-RPC app-server fixture"]
fn fake_app_server_fixture() {
    let Ok(scenario) = std::env::var(FIXTURE_ENV) else {
        return;
    };
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::var_os(METHODS_ENV).unwrap())
        .unwrap();
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let method = request["method"].as_str().unwrap_or_default();
        writeln!(log, "{method}").unwrap();
        log.flush().unwrap();
        let Some(id) = request.get("id") else {
            continue;
        };
        if method == "thread/start" && scenario == "unknown-start-result" {
            continue;
        }
        let result = match method {
            "initialize" => json!({"userAgent":"new-thread-fixture/1"}),
            "thread/start" => json!({"thread":{"id":"new-thread","turns":[]}}),
            _ => json!({}),
        };
        serde_json::to_writer(&mut stdout, &json!({"id":id,"result":result})).unwrap();
        writeln!(stdout).unwrap();
        stdout.flush().unwrap();
    }
}
