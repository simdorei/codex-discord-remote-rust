//! Structured non-blocking questions are messages, not pending JSON-RPC requests.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsyncQuestion {
    pub title: String,
    #[serde(default, deserialize_with = "nullable_options")]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsyncQuestions {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub text: String,
    pub questions: Vec<AsyncQuestion>,
}

fn nullable_options<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

/// Classification must not depend on whether the choices can be rendered.
#[must_use]
pub fn is_async_message(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some("agentMessage" | "agent_message")
    ) && item.get("delivery").and_then(Value::as_str) == Some("async")
}

pub fn parse(params: &Value) -> Result<Option<AsyncQuestions>, String> {
    let Some(item) = params.get("item").filter(|item| is_async_message(item)) else {
        return Ok(None);
    };
    let field = |value: &Value, key: &str| -> Result<String, String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("async question missing {key}"))
    };
    let mut text = item
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let decoded = match item.get("questions") {
        Some(Value::Array(_)) => serde_json::from_value(item["questions"].clone())
            .map_err(|error| format!("invalid async questions: {error}")),
        None | Some(Value::Null) => Ok(Vec::new()),
        _ => Err("invalid async questions: expected an array".into()),
    };
    let questions = match decoded {
        Ok(questions) => questions,
        Err(error) => {
            // Explicit unsupported UI: retain the producer text/metadata, but
            // never invent choices or mistake this malformed question for Final.
            text = format!(
                "{text}\n질문 형식 오류: {error}\n원문 질문 데이터: {}",
                item["questions"]
            );
            Vec::new()
        }
    };
    Ok(Some(AsyncQuestions {
        thread_id: field(params, "threadId")?,
        turn_id: field(params, "turnId")?,
        item_id: field(item, "id")?,
        text,
        questions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn equal_labels_preserve_distinct_question_positions_and_original_identity() {
        let parsed = parse(&json!({"threadId":"t", "turnId":"original", "item":{
            "id":"call_123", "type":"agentMessage", "delivery":"async", "phase":"final_answer",
            "text":"질문", "questions":[{"title":"프로젝트 A?", "options":["허용", "보류"]},
                {"title":"프로젝트 B?", "options":["허용", "보류"]}]
        }}))
        .unwrap()
        .unwrap();
        assert_eq!(parsed.turn_id, "original");
        assert_eq!(parsed.item_id, "call_123");
        assert_eq!(parsed.questions.len(), 2);
        assert_eq!(parsed.questions[1].title, "프로젝트 B?");
        assert_eq!(parsed.questions[1].options, ["허용", "보류"]);
    }

    #[test]
    fn nullable_free_text_options_preserve_question_instead_of_losing_it() {
        let parsed = parse(&json!({"threadId":"t","turnId":"r","item":{
            "id":"q","type":"agentMessage","delivery":"async","text":"입력해주세요",
            "questions":[{"title":"추가 요청은?","options":null}]
        }}))
        .unwrap()
        .unwrap();
        assert_eq!(parsed.questions[0].title, "추가 요청은?");
        assert!(parsed.questions[0].options.is_empty());
    }

    #[test]
    fn malformed_choices_keep_original_text_with_an_explicit_format_error() {
        let parsed = parse(&json!({"threadId":"t","turnId":"r","item":{
            "id":"q","type":"agentMessage","delivery":"async","text":"이 질문은 보존해야 합니다",
            "questions":[{"title":"계속할까요?","options":[{"label":"예"}]}]
        }}))
        .unwrap()
        .unwrap();
        assert!(parsed.questions.is_empty());
        assert!(parsed.text.contains("이 질문은 보존해야 합니다"));
        assert!(parsed.text.contains("질문 형식 오류"));
        assert!(parsed.text.contains("계속할까요?"));
    }
}
