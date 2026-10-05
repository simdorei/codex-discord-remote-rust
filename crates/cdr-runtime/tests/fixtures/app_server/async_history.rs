use super::{Logging, Path, Result, env, json, method, reply, serve, wait_file};

pub(super) fn run() -> Result {
    let log = env("CDR_ASYNC_HISTORY_LOG");
    let script: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(env("CDR_ASYNC_HISTORY_SCRIPT"))?)?;
    serve(Path::new(&log), Logging::Requests, |request| {
        let params = &request["params"];
        let response = match method(&request) {
            "initialize" => json!({"userAgent":"async-history-fixture"}),
            "thread/read" => json!({"thread":{
                "id":script.get("wrong_thread").unwrap_or(&params["threadId"]),
                "status":script.get("thread_status").cloned().unwrap_or_else(||json!({"type":"idle"})),"turns":[]
            }}),
            "thread/goal/get" => {
                if script["terminal_timeout"] == true {
                    return Ok(());
                }
                if script["terminal_gate"] == true {
                    std::fs::write(format!("{log}.terminal-entered"), b"terminal read")?;
                    wait_file(Path::new(&format!("{log}.terminal-release")), 10, true)?;
                }
                script.get("goal_result").cloned().unwrap_or_else(
                    || json!({"goal":{"threadId":params["threadId"],"status":"active"}}),
                )
            }
            "thread/turns/list" => {
                if script["timeout"] == true {
                    return Ok(());
                }
                if script["gate"] == true {
                    std::fs::write(format!("{log}.entered"), b"history read")?;
                    wait_file(Path::new(&format!("{log}.release")), 10, true)?;
                }
                let index = params["cursor"]
                    .as_str()
                    .and_then(|s| s.strip_prefix("page-"))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                if script["endless"] == true {
                    json!({"data":[{"id":format!("unrelated-{index}"),"status":"completed","items":[]}],
                        "nextCursor":format!("page-{}",index+1)})
                } else {
                    script["pages"]
                        .get(index)
                        .cloned()
                        .ok_or("unexpected history page")?
                }
            }
            _ => return Err(format!("unexpected mutation or RPC: {}", method(&request)).into()),
        };
        reply(&request, &response)
    })
}
