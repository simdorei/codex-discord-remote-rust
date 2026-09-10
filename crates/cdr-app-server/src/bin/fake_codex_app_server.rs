use std::io::{self, BufRead, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut delayed = None;
    for raw in stdin.lock().lines() {
        let raw = raw?;
        let message: Value = serde_json::from_str(&raw).map_err(io::Error::other)?;
        if !handle_message(&mut stdout, &message, &mut delayed)? {
            break;
        }
    }
    close_barrier(&mut stdout)?;
    Ok(())
}

fn handle_message(
    mut stdout: &mut impl Write,
    message: &Value,
    delayed: &mut Option<(Value, Value)>,
) -> io::Result<bool> {
    let Some(id) = message.get("id").cloned() else {
        return Ok(true);
    };
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match method {
        "initialize" => send(
            &mut stdout,
            json!({"id": id, "result": {"userAgent": "fake/1"}}),
        )?,
        "test/notify" => {
            send_turn_started(&mut stdout)?;
            send(&mut stdout, json!({"id": id, "result": {"emitted": true}}))?;
        }
        "test/complete" => {
            send_turn_completed(&mut stdout)?;
            send(&mut stdout, json!({"id": id, "result": {}}))?;
        }
        "test/startThenExit" => {
            send_turn_started(&mut stdout)?;
            send(&mut stdout, json!({"id": id, "result": {}}))?;
            return Ok(false);
        }
        "test/delayedEcho" => {
            *delayed = Some((id, message.get("params").cloned().unwrap_or_default()));
            send(
                &mut stdout,
                json!({"method": "test/delayedEntered", "params": {}}),
            )?;
        }
        "test/releaseDelayed" => {
            if let Some((delayed_id, result)) = delayed.take() {
                send(&mut stdout, json!({"id": delayed_id, "result": result}))?;
            }
            send(&mut stdout, json!({"id": id, "result": {}}))?;
        }
        "test/env" => {
            let name = message
                .get("params")
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            send(
                &mut stdout,
                json!({"id": id, "result": {"value": std::env::var(name).ok()}}),
            )?;
        }
        "test/requestApproval" => {
            send_approval(&mut stdout, &json!("server-approval"), "echo safe")?;
            send(
                &mut stdout,
                json!({"id": id, "result": {"requested": true}}),
            )?;
        }
        "test/requestApprovalThenExit" => {
            send_approval(&mut stdout, &json!("server-approval"), "echo unsafe")?;
            send(&mut stdout, json!({"id": id, "result": {}}))?;
            return Ok(false);
        }
        "test/requestApprovalDuplicate" => {
            send_approval(&mut stdout, &json!("server-approval"), "echo safe")?;
            send_approval(&mut stdout, &json!("server-approval"), "echo safe")?;
            send(&mut stdout, json!({"id": id, "result": {}}))?;
        }
        "test/requestApprovalConflict" => {
            send_approval(&mut stdout, &json!("server-approval"), "echo safe")?;
            send_approval(&mut stdout, &json!("server-approval"), "echo unsafe")?;
            send(&mut stdout, json!({"id": id, "result": {}}))?;
        }
        "test/requestTypedIds" => {
            send_approval(&mut stdout, &json!("1"), "echo string")?;
            send_approval(&mut stdout, &json!(1), "echo integer")?;
            send(&mut stdout, json!({"id": id, "result": {}}))?;
        }
        "test/invalid" => {
            writeln!(stdout, "not-json")?;
            stdout.flush()?;
            send(&mut stdout, json!({"id": id, "result": {}}))?;
        }
        "test/timeout" => {}
        _ if id == "server-approval" => {
            send(
                &mut stdout,
                json!({"method": "serverRequest/resolved", "params": {"id": id}}),
            )?;
        }
        _ => send(
            &mut stdout,
            json!({"id": id, "result": message.get("params").cloned().unwrap_or_else(|| json!({}))}),
        )?,
    }
    Ok(true)
}

fn send_turn_started(stdout: &mut impl Write) -> io::Result<()> {
    send(
        stdout,
        json!({"method": "turn/started", "params": {
            "threadId": "thread-a", "turn": {"id": "turn-a"}
        }}),
    )
}

fn send_turn_completed(stdout: &mut impl Write) -> io::Result<()> {
    send(
        stdout,
        json!({"method": "turn/completed", "params": {
            "threadId": "thread-a", "turn": {
                "id": "turn-a", "status": "completed", "error": null
            }
        }}),
    )
}

fn close_barrier(stdout: &mut impl Write) -> io::Result<()> {
    if std::env::var_os("CDR_FAKE_LATE_INGRESS_ON_CLOSE").is_some() {
        send_approval(stdout, &json!("late-server-approval"), "echo late")?;
    }
    let Some(entered) = std::env::var_os("CDR_FAKE_CLOSE_ENTERED_FILE") else {
        return Ok(());
    };
    std::fs::write(entered, b"entered")?;
    let release = std::env::var_os("CDR_FAKE_CLOSE_RELEASE_FILE")
        .ok_or_else(|| io::Error::other("missing close release file"))?;
    while !Path::new(&release).exists() {
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn send_approval(stdout: &mut impl Write, id: &Value, command: &str) -> io::Result<()> {
    send(
        stdout,
        json!({
            "id": id,
            "method": "item/commandExecution/requestApproval",
            "params": {"threadId": "thread-a", "turnId": "turn-a", "command": command}
        }),
    )
}

#[allow(clippy::needless_pass_by_value)] // Test fixture call sites intentionally pass temporary JSON values.
fn send(stdout: &mut impl Write, value: Value) -> io::Result<()> {
    serde_json::to_writer(&mut *stdout, &value).map_err(io::Error::other)?;
    writeln!(stdout)?;
    stdout.flush()
}
