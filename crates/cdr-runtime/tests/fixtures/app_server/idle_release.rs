//! Wire fault fixture: methods are logged before the configured response loss.
use super::{Logging, Result, emit, env, json, method, reply, serve, thread, turn};

pub(super) fn run() -> Result {
    let dir = std::path::PathBuf::from(env("IDLE_TEST_DIR"));
    let mut pending_emitted = false;
    serve(&dir.join("rpc.jsonl"), Logging::All, |request| {
        let mode = std::fs::read_to_string(dir.join("mode")).unwrap_or_default();
        let target = thread(&request);
        match method(&request) {
            "initialize" => reply(&request, &json!({"userAgent":"idle-fixture"})),
            "thread/goal/get" if mode == "drop_goal" => Ok(()),
            "thread/read" if mode == "drop_read" => Ok(()),
            "thread/goal/get" => reply(
                &request,
                &match mode.as_str() {
                    "bad_goal" => json!({}),
                    "paused_goal" => json!({"goal":{"threadId":target,"status":"paused"}}),
                    _ => json!({"goal":null}),
                },
            ),
            "thread/read" => {
                if !pending_emitted
                    && matches!(mode.as_str(), "pending_scoped" | "pending_unscoped")
                {
                    pending_emitted = true;
                    let params = if mode == "pending_scoped" {
                        json!({"threadId":target,"turnId":"T1"})
                    } else {
                        json!({"questions":[]})
                    };
                    emit(
                        &json!({"method":"item/tool/requestUserInput","id":"held-request","params":params}),
                    )?;
                }
                reply(
                    &request,
                    &json!({"thread":{"id":target,
                "status":{"type":if dir.join("unloaded").exists(){"notLoaded"}else{"idle"}},
                "turns":[{"id":if mode=="wrong_turn"{"OTHER"}else{"T1"},"status":"completed","items":[]}]}}),
                )
            }
            "thread/unsubscribe" if mode == "drop_unsub" => Ok(()),
            "thread/unsubscribe" if mode == "bad_unsub" => {
                reply(&request, &json!({"status":"unexpected"}))
            }
            "thread/unsubscribe" => reply(&request, &json!({"status":"unsubscribed"})),
            "thread/resume" if mode == "drop_resume" => Ok(()),
            "thread/resume" => reply(&request, &json!({"thread":{"id":target,"turns":[]}})),
            "turn/start" => {
                reply(&request, &json!({"turn":{"id":"T2"}}))?;
                turn(target, "T2", false)
            }
            "test/pending" => {
                emit(
                    &json!({"method":"item/tool/requestUserInput","id":"question-B",
                    "params":{"threadId":"B","turnId":"T2","questions":[]}}),
                )?;
                reply(&request, &json!({}))
            }
            "test/terminal" => {
                turn(target, "T1", true)?;
                reply(&request, &json!({}))
            }
            "test/closed" => {
                emit(&json!({"method":"thread/closed","params":{"threadId":target}}))?;
                reply(&request, &json!({}))
            }
            _ => reply(&request, &json!({})),
        }
    })
}
