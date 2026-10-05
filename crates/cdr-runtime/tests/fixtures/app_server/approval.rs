use super::{Logging, Path, Result, emit, env, error, json, method, reply, serve, turn};

pub(super) fn run(interaction: bool) -> Result {
    let log = env(if interaction {
        "CDR_INTERACTION_RPC_LOG"
    } else {
        "CDR_APPROVAL_TEST_LOG"
    });
    serve(Path::new(&log), Logging::All, |request| {
        let params = &request["params"];
        let result = match method(&request) {
            "initialize" => {
                json!({"userAgent":if interaction {"worker-test/1"} else {"approval-test/1"}})
            }
            "test/requestApproval" if interaction => {
                turn("thread-a", "turn-a", false)?;
                emit(
                    &json!({"id":"server-approval","method":"item/commandExecution/requestApproval","params":{"threadId":"thread-a","turnId":"turn-a","command":"echo safe"}}),
                )?;
                json!({"requested":true})
            }
            "test/pending" if !interaction => {
                let thread = params["threadId"].as_str().unwrap_or("thread-b");
                let turn_id = params["turnId"].as_str().unwrap_or("turn-b");
                turn(thread, turn_id, false)?;
                emit(
                    &json!({"id":params.get("requestId").cloned().unwrap_or(json!("approval-1")),
                    "method":params.get("method").cloned().unwrap_or(json!("item/commandExecution/requestApproval")),
                    "params":{"threadId":thread,"turnId":turn_id,
                        "command":params.get("command").cloned().unwrap_or(json!("echo fixture")),
                        "questions":params.get("questions").cloned().unwrap_or(json!([{"id":"q","question":"Choose one","options":[{"label":"First"},{"label":"Second"}]}]))}}),
                )?;
                json!({})
            }
            "test/finish" if !interaction => {
                turn("thread-b", "turn-b", true)?;
                json!({})
            }
            _ if interaction => json!({}),
            _ => return error(&request, -32601, "unexpected test RPC"),
        };
        reply(&request, &result)
    })
}
