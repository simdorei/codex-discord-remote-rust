use super::{
    Duration, Logging, Path, Result, conflicting_identity, env, error, json, method, reply, serve,
    wait_file,
};

pub(super) fn run() -> Result {
    let scenario = env("RESUME_SCENARIO");
    let log = env("RESUME_LOG");
    let mut resumed = false;
    serve(Path::new(&log), Logging::Requests, |request| {
        let result = match method(&request) {
            "initialize" => json!({"userAgent":"resume-contract/1"}),
            "thread/resume" => {
                resumed = true;
                if scenario == "writer" {
                    return error(
                        &request,
                        -32600,
                        "thread thread-b already has an active writer",
                    );
                }
                let mut result = json!({"thread":{"id":if scenario == "wrong_resume" {"other"} else {"thread-b"}}});
                conflicting_identity(&mut result, &scenario, "resume", "thread-b");
                result
            }
            "thread/read" => {
                if scenario == "slow_read" {
                    std::thread::sleep(Duration::from_secs(5));
                }
                if scenario == "gated_read" {
                    wait_file(Path::new(&format!("{log}.release")), 40, true)?;
                }
                let status = match scenario.as_str() {
                    "gated_read" => "idle",
                    "idle" | "active" | "systemError" => scenario.as_str(),
                    "unknown_status" => "futureUnknown",
                    _ if resumed && scenario != "still_unloaded" => "idle",
                    _ => "notLoaded",
                };
                let mut result = json!({"thread":{"id":if scenario == "wrong_read" {"other"} else {"thread-b"},"status":{"type":status}}});
                if scenario == "missing_status" {
                    result["thread"].as_object_mut().unwrap().remove("status");
                }
                conflicting_identity(&mut result, &scenario, "read", "thread-b");
                result
            }
            _ => json!({}),
        };
        reply(&request, &result)
    })
}
