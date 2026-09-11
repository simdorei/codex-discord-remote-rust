use super::{Logging, Path, Result, env, error, json, method, reply, serve, thread};

pub(super) fn run() -> Result {
    let mode = env("DISPLAY_MODE");
    serve(
        Path::new(&env("DISPLAY_LOG")),
        Logging::Requests,
        |request| {
            let thread = thread(&request);
            let result = match method(&request) {
                "initialize" => json!({"userAgent":"display-fixture"}),
                "thread/read" => {
                    let mut result = json!({"thread":{"id":if mode == "wrong_id" {"wrong"} else {thread},"status":{"type":"idle"}}});
                    if mode == "conflicting_id" {
                        result["threadId"] = json!(thread);
                        result["thread"]["id"] = json!("wrong");
                    }
                    if mode == "missing_nested_id" {
                        result["conversationId"] = json!(thread);
                        result["thread"].as_object_mut().unwrap().remove("id");
                    }
                    result
                }
                "thread/goal/get" => match mode.as_str() {
                    "goal_hang" => return Ok(()),
                    "goal_error" => return error(&request, -32001, "fixture goal lookup failed"),
                    "goal_missing" => json!({}),
                    _ => json!({"goal":null}),
                },
                _ => {
                    return error(
                        &request,
                        -32601,
                        "unexpected mutating method in display fixture",
                    );
                }
            };
            reply(&request, &result)
        },
    )
}
