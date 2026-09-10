//! !new without text reserves the next authorized human message in the same
//! room. Consumption and the resulting immutable ingress share one transaction.
use super::{IngressKind, NewIngress};
use crate::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// Only changes the ordinary-message mention gate. Actual consumption still
/// occurs under the admission transaction, after the unchanged access checks.
pub fn pending_new_prompt(
    path: &std::path::Path,
    channel: i64,
    user: i64,
    event: i64,
) -> Result<Option<String>> {
    let connection = crate::schema::open_initialized(path)?;
    let row: Option<(String, bool)> = connection.query_row(
        "SELECT ingress_id,event_id < ? AND json_extract(payload_json,'$.new_prompt_arm.consumed_by') IS NULL
         FROM discord_ingress_journal WHERE channel_id=? AND owner_user_id=? AND kind='message'
         AND json_type(payload_json,'$.new_prompt_arm')='object' ORDER BY event_id DESC LIMIT 1",
        params![event,channel,user],|row|Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    Ok(row.and_then(|(id, pending)| pending.then_some(id)))
}

pub(super) fn prepare_in(connection: &Connection, original: &NewIngress) -> Result<NewIngress> {
    let mut request = original.clone();
    if request.kind != IngressKind::Message
        || request.payload["version"] != 1
        || request.payload["author_is_bot"] != false
    {
        return Ok(request);
    }
    let raw = request.payload["content"].as_str().unwrap_or("").trim();
    if request
        .payload
        .pointer("/plan/Execute/New/prompt")
        .and_then(Value::as_str)
        == Some("")
        && raw.eq_ignore_ascii_case("!new")
    {
        request.payload["new_prompt_arm"] = json!({"consumed_by":null});
        request.payload["plan"] =
            json!({"Respond":"새 대화를 준비했습니다. 같은 방에 첫 요청을 보내주세요."});
        request.target_thread_id = None;
        return Ok(request);
    }
    if raw.starts_with('!')
        || request
            .payload
            .pointer("/plan/Execute/Ask/prompt")
            .is_none()
    {
        return Ok(request);
    }
    let pending: Option<(String, i64, String)> = connection
        .query_row(
            "SELECT ingress_id,event_id,payload_json FROM discord_ingress_journal
         WHERE channel_id=? AND owner_user_id=? AND kind='message'
           AND json_type(payload_json,'$.new_prompt_arm')='object'
         ORDER BY event_id DESC LIMIT 1",
            params![request.channel_id, request.owner_user_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((arm_id, arm_event, payload)) = pending else {
        return Ok(without_consumption(request));
    };
    let arm: Value = serde_json::from_str(&payload)?;
    if request.event_id.is_none_or(|id| id <= arm_event)
        || !arm["new_prompt_arm"]["consumed_by"].is_null()
        || request.payload["new_prompt_mention_arm"]
            .as_str()
            .is_some_and(|id| id != arm_id)
    {
        return Ok(without_consumption(request));
    }
    connection.execute(
        "UPDATE discord_ingress_journal SET payload_json=json_set(payload_json,'$.new_prompt_arm.consumed_by',?),updated_at=? WHERE ingress_id=?",
        params![request.ingress_id,request.now,arm_id],
    )?;
    request.payload["new_prompt_arm_ref"] = json!(arm_id);
    request.target_thread_id = None;
    if request.payload.get("new_origin").is_none() {
        request.payload["new_origin"] = serde_json::to_value(
            crate::mapping::new_thread_origin_in(connection, request.channel_id)?,
        )?;
    }
    if arm["routing"] == request.payload["routing"]
        && arm["new_origin"] == request.payload["new_origin"]
    {
        let prompt = request.payload["plan"]["Execute"]["Ask"]["prompt"].clone();
        request.payload["plan"] = json!({"Execute":{"New":{"prompt":prompt}}});
    } else {
        request.payload["plan"] = json!({"Respond":"ERROR: !new 이후 방 연결이 변경되었습니다. 실행하지 않았습니다. !new를 다시 입력해주세요."});
    }
    Ok(request)
}

fn without_consumption(mut request: NewIngress) -> NewIngress {
    if request.payload["new_prompt_mention_arm"].is_string() {
        request.target_thread_id = None;
        request.payload["plan"] = json!({"Respond":"ERROR: !new 예약이 이미 사용되었거나 변경되어 이 글은 실행하지 않았습니다. !new를 다시 입력하거나 필요한 멘션을 포함해주세요."});
    }
    request
}
