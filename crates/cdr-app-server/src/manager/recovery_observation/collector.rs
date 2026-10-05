use super::{AppServerError, ReadLifetime, invalid};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const MAX_BYTES: usize = 1_048_576;
struct History {
    turns: Vec<Value>,
    exhausted: bool,
}

fn charge(value: &Value, bytes: &mut usize) -> Result<(), AppServerError> {
    *bytes = bytes.saturating_add(value.to_string().len());
    if *bytes > MAX_BYTES {
        return Err(invalid("recovery observation exceeds its byte bound"));
    }
    Ok(())
}

fn exact_idle(value: &Value, thread: &str) -> Result<(), AppServerError> {
    if value["thread"]["id"] != thread
        || value["thread"]["status"]["type"] != "idle"
        || value["thread"]["archived"] == true
        || value
            .get("truncated")
            .is_some_and(|v| v != &Value::Bool(false))
    {
        return Err(invalid(
            "recovery observation needs the exact current idle thread",
        ));
    }
    Ok(())
}

fn ended_goal(value: &Value, thread: &str) -> Result<(), AppServerError> {
    let goal = value
        .get("goal")
        .ok_or_else(|| invalid("current Goal observation is missing"))?;
    if !(goal.is_null() || goal["threadId"] == thread && goal["status"] == "complete") {
        return Err(invalid("current Goal has not ended for the exact target"));
    }
    Ok(())
}

pub(super) async fn collect(pin: &ReadLifetime) -> Result<Value, AppServerError> {
    let mut bytes = 0;
    let params = json!({"threadId":pin.thread,"includeTurns":false});
    let initial = pin.request("thread/read", params.clone()).await?;
    charge(&initial, &mut bytes)?;
    exact_idle(&initial, &pin.thread)?;
    let history = history(pin, &mut bytes).await?;
    let goal = pin
        .request("thread/goal/get", json!({"threadId":pin.thread}))
        .await?;
    charge(&goal, &mut bytes)?;
    ended_goal(&goal, &pin.thread)?;
    let thread = pin.request("thread/read", params).await?;
    charge(&thread, &mut bytes)?;
    exact_idle(&thread, &pin.thread)?;
    // Completeness is only for the requested owner set. A remaining cursor does
    // not certify that the whole thread's historical inventory was exhausted.
    let result = json!({"threadId":pin.thread,"truncated":false,
        "required_owners_complete":true,"history_exhausted":history.exhausted,
        "turns":history.turns,"goal_observation":goal,"thread_observation":thread});
    if result.to_string().len() > MAX_BYTES {
        return Err(invalid(
            "combined recovery observation exceeds its byte bound",
        ));
    }
    Ok(result)
}

async fn history(pin: &ReadLifetime, bytes: &mut usize) -> Result<History, AppServerError> {
    let mut found = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut cursors = BTreeSet::new();
    let mut cursor = Value::Null;
    let mut turns = Vec::new();
    for _ in 0..8 {
        let page = pin
            .request(
                "thread/turns/list",
                json!({"threadId":pin.thread,
            "limit":16,"sortDirection":"desc","itemsView":"full","cursor":cursor}),
            )
            .await?;
        charge(&page, bytes)?;
        if page
            .get("truncated")
            .is_some_and(|v| v != &Value::Bool(false))
            || page
                .get("threadId")
                .is_some_and(|v| v.as_str() != Some(pin.thread.as_str()))
        {
            return Err(invalid(
                "recovery history is truncated or belongs to another target",
            ));
        }
        let data = page
            .get("data")
            .and_then(Value::as_array)
            .filter(|v| v.len() <= 16)
            .ok_or_else(|| invalid("recovery history has no bounded turn array"))?;
        for turn in data {
            let id = turn
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 512)
                .ok_or_else(|| invalid("recovery turn identity is missing"))?;
            if !seen.insert(id.to_owned()) {
                return Err(invalid("duplicate recovery turn identity"));
            }
            if pin.owners.contains(id) {
                if !matches!(
                    turn["status"].as_str(),
                    Some("completed" | "failed" | "interrupted")
                ) || turn
                    .get("truncated")
                    .is_some_and(|v| v != &Value::Bool(false))
                {
                    return Err(invalid("required original execution is not fully terminal"));
                }
                found.insert(id.to_owned());
                turns.push(turn.clone());
            }
        }
        let next = match page.get("nextCursor") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(value)) if !value.is_empty() && value.len() <= 2048 => {
                Value::String(value.clone())
            }
            _ => return Err(invalid("invalid recovery history cursor")),
        };
        if found == pin.owners {
            return Ok(History {
                turns,
                exhausted: page.get("nextCursor") == Some(&Value::Null),
            });
        }
        if next.is_null() {
            return Err(invalid(
                "required original execution is absent from bounded history",
            ));
        }
        if !cursors.insert(next.to_string()) {
            return Err(invalid("recovery history cursor repeated"));
        }
        cursor = next;
    }
    Err(invalid(
        "recovery history exceeds eight pages; absence is not proof",
    ))
}
