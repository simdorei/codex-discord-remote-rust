use super::{
    Duration, Instant, Logging, Path, Result, Value, emit, env, error, json, method, reply, serve,
    thread, wait_file,
};

fn observe(thread: &str, settings: &Value) -> Result {
    emit(
        &json!({"method":"thread/settings/updated","params":{"threadId":thread,"threadSettings":settings}}),
    )
}

pub(super) fn run() -> Result {
    let mode = env("SETTINGS_TEST_MODE");
    let log = env("SETTINGS_TEST_LOG");
    let root = Path::new(&log).parent().unwrap();
    let mut settings = json!({"model":"model-a","effort":"high","serviceTier":null});
    serve(Path::new(&log), Logging::All, |request| {
        let thread = thread(&request);
        let result = match method(&request) {
            "initialize" => json!({"userAgent":"settings-test/1"}),
            "test/observe" => {
                observe(thread, &settings)?;
                json!({})
            }
            "test/pauseReads" | "test/blocked" => json!({}),
            "model/list" => json!({"data":[
                {"model":"model-a","displayName":"Model A","supportedReasoningEfforts":[{"reasoningEffort":"high"},{"reasoningEffort":"low"}]},
                {"model":"model-b","displayName":"Model B","supportedReasoningEfforts":[{"reasoningEffort":"medium"}]}]}),
            "thread/read" => json!({"thread":{"id":thread,"status":{"type":"idle"},"turns":[]}}),
            "thread/resume" => {
                let mut result = json!({"thread":{"id":thread,"status":{"type":"idle"},"turns":[]},"model":settings["model"],"reasoningEffort":settings["effort"],"serviceTier":settings["serviceTier"]});
                if mode == "wrong-thread" {
                    result["thread"]["id"] = json!("other-thread");
                }
                if mode == "incomplete-resume" {
                    result.as_object_mut().unwrap().remove("serviceTier");
                }
                if mode == "watermark" {
                    std::fs::write(root.join("resume-ready"), "ready")?;
                    wait_file(&root.join("resume-release"), 40, true)?;
                }
                result
            }
            "thread/settings/update" => {
                if mode == "reject" {
                    return error(&request, -32600, "fixture settings rejection");
                }
                for key in ["model", "effort", "serviceTier"] {
                    if let Some(value) = request["params"].get(key) {
                        settings[key] = value.clone();
                    }
                }
                if mode == "mismatch" {
                    settings["model"] = json!("model-a");
                }
                if !matches!(mode.as_str(), "missing" | "watermark") {
                    observe(thread, &settings)?;
                }
                json!({})
            }
            _ => return error(&request, -32601, "unexpected fixture method"),
        };
        reply(&request, &result)?;
        if method(&request) == "thread/resume" && mode == "watermark" {
            observe(thread, &settings)?;
            let deadline = Instant::now() + Duration::from_secs(40);
            let mut emitted = false;
            while !root.join("writer-release").exists() {
                if !emitted && root.join("premature-emit").exists() {
                    settings["model"] = json!("model-b");
                    observe(thread, &settings)?;
                    emitted = true;
                }
                if Instant::now() >= deadline {
                    return Err("settings writer fixture gate timed out".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if method(&request) == "test/pauseReads" {
            wait_file(
                Path::new(
                    request["params"]["releasePath"]
                        .as_str()
                        .ok_or("missing releasePath")?,
                ),
                40,
                true,
            )?;
        }
        Ok(())
    })
}
