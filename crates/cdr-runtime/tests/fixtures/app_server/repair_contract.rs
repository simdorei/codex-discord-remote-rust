use super::{Logging, Path, Result, env, json, method, reply, serve, wait_file};
use std::collections::BTreeMap;

pub(super) fn run() -> Result {
    let log = env("CDR_REPAIR_RPC_LOG");
    let mode = env("CDR_REPAIR_MODE");
    let mut sessions = BTreeMap::from([
        ("thread-a".to_owned(), "protected-other-session".to_owned()),
        ("thread-b".to_owned(), "old-target-session".to_owned()),
    ]);
    let mut late_reset = None;
    serve(Path::new(&log), Logging::Requests, |request| {
        let params = &request["params"];
        let target = params["threadId"].as_str().unwrap_or("");
        let value = match method(&request) {
            "initialize" => json!({"userAgent":"repair-fixture"}),
            "thread/read" => {
                if params["fixtureReleaseReset"] == true
                    && let Some(pending) = late_reset.take()
                {
                    reply(
                        &pending,
                        &json!({"content":[{"type":"text","text":"js kernel reset"}]}),
                    )?;
                }
                json!({"thread":{"id":params["threadId"],"status":{"type":if mode == "busy" { "active" } else { "idle" }},"turns":[],
                    "fixtureSession":sessions.get(target),"fixturePid":std::process::id()}})
            }
            "mcpServerStatus/list" if mode == "missing" => json!({"data":[],"nextCursor":null}),
            "mcpServerStatus/list" => {
                if mode == "gate-inventory" {
                    std::fs::write(format!("{log}.entered"), b"inventory")?;
                    wait_file(Path::new(&format!("{log}.release")), 10, true)?;
                }
                json!({"data":[{"name":"node_repl","runtimeStatus":"connected","tools":{"js":{"name":"js"},"js_reset":{"name":"js_reset"}}}],"nextCursor":null})
            }
            "mcpServer/tool/call" if params["tool"] == "js_reset" => {
                sessions.remove(target);
                if mode == "timeout" {
                    return Ok(());
                }
                if mode == "late-reset" {
                    late_reset = Some(request.clone());
                    return Ok(());
                }
                if mode == "reset-error" {
                    json!({"isError":true,"content":[{"type":"text","text":"fixture reset failed"}]})
                } else if mode == "reset-format" {
                    json!({"content":[{"type":"text","text":"unconfirmed format"}]})
                } else {
                    json!({"content":[{"type":"text","text":"js kernel reset"}]})
                }
            }
            "mcpServer/tool/call"
                if params["arguments"]["code"]
                    .as_str()
                    .unwrap_or("")
                    .contains("list_apps") =>
            {
                match mode.as_str() {
                    "pipe" => {
                        json!({"isError":true,"content":[{"type":"text","text":"Computer Use native pipe is unavailable"}]})
                    }
                    "probe-error" => {
                        json!({"isError":true,"content":[{"type":"text","text":"fixture probe denied"}]})
                    }
                    "probe-format" => {
                        json!({"content":[{"type":"text","text":"{\"cdrRepair\":\"unknown\"}"}]})
                    }
                    _ => {
                        json!({"content":[{"type":"text","text":"{\"cdrRepair\":\"ready\",\"apps\":2}"}]})
                    }
                }
            }
            "mcpServer/tool/call" => match mode.as_str() {
                "init-error" => {
                    json!({"isError":true,"content":[{"type":"text","text":"fixture initialization denied"}]})
                }
                "init-malformed" => json!({"isError":"false","content":[]}),
                "init-no-content" => json!({}),
                _ => json!({"content":[]}),
            },
            _ => return Err(format!("unexpected repair RPC: {}", method(&request)).into()),
        };
        reply(&request, &value)
    })
}
