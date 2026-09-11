use super::{Logging, Path, Result, env, error, json, method, reply, serve};

pub(super) fn run() -> Result {
    let mode = env("USAGE_TEST_MODE");
    serve(
        Path::new(&env("USAGE_TEST_LOG")),
        Logging::Methods,
        |request| {
            let result = match method(&request) {
                "initialize" => json!({"userAgent":"usage-fixture"}),
                "account/rateLimits/read" => {
                    if mode == "rates_error" {
                        return error(&request, -32000, "fixture rate lookup failed");
                    }
                    json!({"rateLimits":{"planType":"pro","primary":{"usedPercent":25,"windowDurationMins":300}}})
                }
                "account/usage/read" => {
                    if mode == "usage_error" {
                        return error(&request, -32001, "fixture usage lookup failed");
                    }
                    if mode == "empty" {
                        json!({"dailyUsageBuckets":[]})
                    } else {
                        let today = chrono::Utc::now().date_naive();
                        json!({"dailyUsageBuckets":[{"startDate":today.to_string(),"tokens":1234},{"startDate":(today-chrono::Duration::days(40)).to_string(),"tokens":999_999}],"summary":{"lifetimeTokens":54_321}})
                    }
                }
                _ => {
                    return error(
                        &request,
                        -32601,
                        "unexpected method in readonly usage fixture",
                    );
                }
            };
            reply(&request, &result)
        },
    )
}
