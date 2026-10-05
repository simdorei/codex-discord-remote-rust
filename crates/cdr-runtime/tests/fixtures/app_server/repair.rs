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
                json!({"data":[{"name":"node_repl","runtimeStatus":"connected","tools":{"js":{"name":"js"},"js_reset":{"name":"js_reset"}}}],"nextCursor":null})
            }
            "mcpServer/tool/call" if params["tool"] == "js_reset" => {
                if mode == "timeout" {
                    return Ok(());
                }
                json!({"content":[{"type":"text","text":"js kernel reset"}]})
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
