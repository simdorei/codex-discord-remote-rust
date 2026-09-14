//! Native replacement for the legacy subprocess fixtures. Not used by the bot.
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

#[path = "app_server/action.rs"]
mod action;
#[path = "app_server/approval.rs"]
mod approval;
#[path = "app_server/archive.rs"]
mod archive;
#[path = "app_server/async_question.rs"]
mod async_question;
#[path = "app_server/display.rs"]
mod display;
#[path = "app_server/goal.rs"]
mod goal;
#[path = "app_server/idle_release.rs"]
mod idle_release;
#[path = "app_server/resume.rs"]
mod resume;
#[path = "app_server/settings.rs"]
mod settings;
#[path = "app_server/usage.rs"]
mod usage;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn run() -> Result {
    match std::env::args().nth(2).as_deref() {
        Some("action") => action::run(),
        Some("async-question") => async_question::run(),
        Some("approval") => approval::run(false),
        Some("interaction") => approval::run(true),
        Some("archive") => archive::run(),
        Some("display") => display::run(),
        Some("goal") => goal::run(),
        Some("idle-release") => idle_release::run(),
        Some("resume") => resume::run(),
        Some("settings") => settings::run(),
        Some("usage") => usage::run(),
        _ => Err("unknown native app-server fixture scenario".into()),
    }
}

#[derive(Clone, Copy)]
enum Logging {
    All,
    Requests,
    Methods,
    PlainMethods,
}

fn serve(log: &Path, logging: Logging, mut handle: impl FnMut(Value) -> Result) -> Result {
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        if matches!(logging, Logging::All) || request.get("id").is_some() {
            if matches!(logging, Logging::PlainMethods) {
                writeln!(
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(log)?,
                    "{}",
                    method(&request)
                )?;
            } else {
                append(
                    log,
                    &if matches!(logging, Logging::Methods) {
                        json!({"method":request["method"]})
                    } else {
                        request.clone()
                    },
                )?;
            }
        }
        if request.get("id").is_some() && request.get("method").is_some() {
            handle(request)?;
        }
    }
    Ok(())
}

fn emit(value: &Value) -> Result {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn append(path: &Path, value: &Value) -> Result {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut record = serde_json::to_vec(value)?;
    record.push(b'\n');
    file.write_all(&record)?;
    Ok(())
}

fn reply(request: &Value, result: &Value) -> Result {
    emit(&json!({"id":request["id"],"result":result}))
}
fn error(request: &Value, code: i32, message: &str) -> Result {
    emit(&json!({"id":request["id"],"error":{"code":code,"message":message}}))
}
fn method(request: &Value) -> &str {
    request["method"].as_str().unwrap_or("")
}
fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("fixture environment missing {name}"))
}
fn thread(request: &Value) -> &str {
    request["params"]["threadId"].as_str().unwrap_or("thread-b")
}
fn turn(thread: &str, id: &str, complete: bool) -> Result {
    emit(
        &json!({"method":if complete {"turn/completed"} else {"turn/started"},"params":{"threadId":thread,"turn":{"id":id,"status":if complete {"completed"} else {"inProgress"}}}}),
    )
}
fn wait_file(path: &Path, seconds: u64, required: bool) -> Result {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !path.exists() {
        if Instant::now() >= deadline {
            return if required {
                Err(format!("fixture gate timed out: {}", path.display()).into())
            } else {
                Ok(())
            };
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
fn conflicting_identity(result: &mut Value, scenario: &str, operation: &str, thread: &str) {
    if scenario == format!("conflicting_{operation}")
        || scenario == format!("missing_nested_{operation}")
    {
        result["threadId"] = json!(thread);
        if scenario.starts_with("conflicting_") {
            result["thread"]["id"] = json!("other");
        } else {
            result["thread"].as_object_mut().unwrap().remove("id");
        }
    }
}
