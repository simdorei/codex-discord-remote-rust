//! Strict wire fixture for async question ownership/chronology/uncertainty tests.
use super::{Logging, Path, Result, emit, env, error, json, method, reply, serve, turn};

pub(super) fn run() -> Result {
    let log = env("CDR_ACTION_RPC_LOG");
    let mut active = false;
    let mut latest = "original".to_owned();
    let mut reads = 0;
    let mut stale_on_second = false;
    let mut drop_reply = false;
    let mut goal = false;
    serve(Path::new(&log), Logging::Requests, |request| {
        let params = &request["params"];
        let result = match method(&request) {
            "initialize" => json!({"userAgent":"async-question-test"}),
            "test/active-turn" => {
                active = true;
                turn("thread-b", "original", false)?;
                json!({})
            }
            "test/finish-turn" => {
                active = false;
                turn("thread-b", "original", true)?;
                json!({})
            }
            "test/stale-on-second-read" => {
                stale_on_second = true;
                reads = 0;
                json!({})
            }
            "test/drop-reply" => {
                drop_reply = true;
                json!({})
            }
            "test/goal-on" => {
                goal = true;
                json!({})
            }
            "test/goal-next-question" => {
                active = true;
                latest = "goal-next".into();
                turn("thread-b", &latest, false)?;
                emit_question(&latest)?;
                json!({})
            }
            "test/question" => {
                emit_question(&latest)?;
                json!({})
            }
            "thread/turns/list" => {
                assert_eq!(params["threadId"], "thread-b");
                assert_eq!(params["sortDirection"], "desc");
                assert_eq!(params["limit"], 1);
                reads += 1;
                if stale_on_second && reads >= 2 {
                    latest = "newer".into();
                }
                json!({"data":[{"id":latest,"status":if active {"inProgress"} else {"completed"},"items":[]}],"nextCursor":null})
            }
            "thread/goal/get" => {
                if goal {
                    json!({"goal":{"threadId":"thread-b","status":"active"}})
                } else {
                    json!({"goal":null})
                }
            }
            "thread/read" => thread_snapshot(goal, &latest, active),
            "turn/steer" => {
                assert!(active);
                assert_eq!(params["threadId"], "thread-b");
                assert_eq!(params["expectedTurnId"], "original");
                if drop_reply {
                    return Ok(());
                }
                json!({"turnId":"original"})
            }
            "turn/start" => {
                assert!(!active);
                assert_eq!(params["threadId"], "thread-b");
                active = true;
                latest = "answer-turn".into();
                if drop_reply {
                    return Ok(());
                }
                json!({"turn":{"id":"answer-turn","status":"inProgress"}})
            }
            _ => {
                return error(
                    &request,
                    -32601,
                    "unsupported async question fixture method",
                );
            }
        };
        reply(&request, &result)
    })
}

fn thread_snapshot(goal: bool, latest: &str, active: bool) -> serde_json::Value {
    let items = if goal {
        vec![json!({"type":"agentMessage","text":"진행 시험","phase":"final_answer"})]
    } else {
        vec![]
    };
    let mut turns = vec![
        json!({"id":latest,"status":if active {"inProgress"} else {"completed"},"items":items}),
    ];
    if goal && latest != "original" {
        turns.insert(0, json!({"id":"original","status":"completed","items":[{"type":"agentMessage","text":"진행 시험","phase":"final_answer"}]}));
    }
    json!({"thread":{"id":"thread-b","turns":turns}})
}

fn emit_question(turn_id: &str) -> Result {
    emit(
        &json!({"method":"item/completed","params":{"threadId":"thread-b","turnId":turn_id,"item":{
            "id":"question-call","type":"agentMessage","phase":"final_answer","delivery":"async","text":"어느 프로젝트인가요?",
            "questions":[{"title":"프로젝트 A?","options":["허용","보류"]},{"title":"프로젝트 B?","options":["허용","보류"]}]
        }}}),
    )
}
