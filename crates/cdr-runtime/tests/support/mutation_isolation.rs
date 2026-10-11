use std::fs::{self, OpenOptions};
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cdr_app_server::{AppServerConfig, ResidentAppServer};
use cdr_runtime::dead_generation_recovery::RuntimeDeadGenerationFence;
use serde_json::{Value, json};

pub const WAIT: Duration = Duration::from_secs(2);
pub const EXPIRY: Duration = Duration::from_millis(250);

pub fn config(log: &Path, mode: &str) -> AppServerConfig {
    let mut config = AppServerConfig::new(std::env::current_exe().unwrap());
    config.arguments = vec![
        "--exact".into(),
        "support::p03b_fixture_child".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    config
        .environment
        .insert("CDR_P03B_LOG".into(), log.to_string_lossy().into_owned());
    config
        .environment
        .insert("CDR_P03B_MODE".into(), mode.into());
    config
}

pub async fn start(db: &Path, log: &Path, owner: &str, mode: &str) -> Arc<ResidentAppServer> {
    let fence = Arc::new(RuntimeDeadGenerationFence::new(db.into(), owner.into(), None).unwrap());
    Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(config(log, mode), fence)
            .await
            .unwrap(),
    )
}

pub fn calls(log: &Path) -> Vec<Value> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

pub fn count(log: &Path, method: &str, target: &str) -> usize {
    calls(log)
        .iter()
        .filter(|r| r["method"] == method && r["params"]["threadId"] == target)
        .count()
}

pub async fn seen(log: &Path, method: &str, target: &str) {
    tokio::time::timeout(WAIT, async {
        while count(log, method, target) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("native child did not receive request within fixed budget");
}

fn send(writer: &mut impl Write, value: &Value) {
    serde_json::to_writer(&mut *writer, value).unwrap();
    writeln!(writer).unwrap();
    writer.flush().unwrap();
}

#[test]
#[ignore = "spawned native JSON-RPC fixture"]
fn p03b_fixture_child() {
    let Some(path) = std::env::var_os("CDR_P03B_LOG") else {
        return;
    };
    let mode = std::env::var("CDR_P03B_MODE").unwrap();
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stdout().lock();
    let mut held = Vec::new();
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).unwrap() == 0 {
            return;
        }
        let request: Value = serde_json::from_str(&line).unwrap();
        send(&mut log, &request);
        let method = request["method"].as_str().unwrap_or_default();
        if method == "initialized" && mode == "partial" {
            let mut bytes = [0_u8; 64];
            input.read_exact(&mut bytes).unwrap();
            send(&mut log, &json!({"method":"fixture/partial","bytes":64}));
            return;
        }
        let Some(id) = request.get("id") else {
            continue;
        };
        let params = &request["params"];
        let target = &params["threadId"];
        if params["fixture"] == "hold" {
            held.push(id.clone());
            continue;
        }
        if params["fixture"] == "late-a" {
            for old in &held {
                for _ in 0..2 {
                    send(&mut output, &json!({"id":old,"result":{"marker":"late-a"}}));
                }
            }
        }
        if params["fixture"] == "remote"
            || (method == "thread/read" && mode == "error-resume-recovery")
        {
            send(
                &mut output,
                &json!({"id":id,"error":{"code":-32000,"message":"fixture rejection"}}),
            );
            continue;
        }
        let result = match method {
            "initialize" => json!({"userAgent":"mutation-isolation-fixture"}),
            "thread/read" if mode == "active-resume-recovery" => {
                json!({"thread":{"id":target,"status":{"type":"active"}}})
            }
            "thread/read" => json!({"thread":{"id":target,"turns":[],"status":{"type":"idle"}}}),
            "thread/turns/list" if mode == "malformed-resume-recovery" => json!({"data":[]}),
            "thread/turns/list" => json!({"data":[],"nextCursor":null}),
            "thread/resume" => json!({"thread":{"id":target}}),
            "turn/start" => {
                json!({"turn":{"id":format!("turn-{}", target.as_str().unwrap_or("new"))}})
            }
            _ => json!({"marker":target}),
        };
        send(&mut output, &json!({"id":id,"result":result}));
    }
}
