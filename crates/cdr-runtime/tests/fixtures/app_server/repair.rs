use super::{Logging, Path, Result, env, json, method, reply, serve};

pub(super) fn run() -> Result {
    let log = env("CDR_REPAIR_RPC_LOG");
    let mode = env("CDR_REPAIR_MODE");
    serve(Path::new(&log), Logging::Requests, |request| {
        let params = &request["params"];
        let value = match method(&request) {
            "initialize" => json!({"userAgent":"repair-fixture"}),
            "thread/read" => {
                json!({"thread":{"id":params["threadId"],"status":{"type":if mode == "busy" { "active" } else { "idle" }},"turns":[]}})
            }
            "mcpServerStatus/list" if mode == "missing" => json!({"data":[],"nextCursor":null}),
            "mcpServerStatus/list" => {
                if mode == "wait_inventory" {
                    let release = Path::new(&log).with_extension("release");
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    while !release.is_file() {
                        if std::time::Instant::now() >= deadline {
                            return Err("repair inventory barrier timed out".into());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
                let legacy = json!({"name":"node_repl","runtimeStatus":"connected","tools":{"js":{"name":"js"},"js_reset":{"name":"js_reset"}}});
                let unified = json!({"name":"cua_repl","runtimeStatus":"connected","tools":{"js":{"name":"js"},"js_reset":{"name":"js_reset"}}});
                if mode == "unified_paged" && params["cursor"].is_null() {
                    return reply(&request, &json!({"data":[legacy],"nextCursor":"second"}));
                }
                if mode.starts_with("unified") {
                    let data = if mode == "unified_both" {
                        vec![legacy, unified]
                    } else {
                        vec![unified]
                    };
                    json!({"data":data,"nextCursor":null})
                } else {
                    json!({"data":[legacy],"nextCursor":null})
                }
            }
            "mcpServer/tool/call" if params["tool"] == "js_reset" => {
                if mode == "timeout" {
                    return Ok(());
                }
                match mode.as_str() {
                    "unified_reset_error" => {
                        json!({"isError":true,"content":[{"type":"text","text":"reset refused"}]})
                    }
                    "unified_reset_malformed" => {
                        json!({"content":[{"type":"text","text":"reset outcome unknown"}]})
                    }
                    _ => json!({"content":[{"type":"text","text":"js kernel reset"}]}),
                }
            }
            "mcpServer/tool/call" if params["server"] == "cua_repl" => {
                // The official standalone worker needs an active model turn.
                // A reset acknowledgement cannot authorize a native/browser probe.
                json!({"isError":true,"content":[{"type":"text","text":"spawn EPERM: active model turn required"}]})
            }
            "mcpServer/tool/call"
                if params["arguments"]["code"]
                    .as_str()
                    .unwrap_or("")
                    .contains("list_apps") =>
            {
                if mode == "pipe" {
                    json!({"isError":true,"content":[{"type":"text","text":"Computer Use native pipe is unavailable"}]})
                } else {
                    json!({"content":[{"type":"text","text":"{\"cdrRepair\":\"ready\",\"apps\":2}"}]})
                }
            }
            "mcpServer/tool/call" => json!({"content":[]}),
            _ => return Err(format!("unexpected repair RPC: {}", method(&request)).into()),
        };
        reply(&request, &value)
    })
}
