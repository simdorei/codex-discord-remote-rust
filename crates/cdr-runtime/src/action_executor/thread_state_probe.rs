use cdr_app_server::{ResidentAppServer, requests::read_thread};
use cdr_codex_state::ThreadInfo;
use std::{collections::BTreeMap, time::Duration};
use tokio::time::Instant;

pub(super) async fn read(
    server: &ResidentAppServer,
    threads: &[ThreadInfo],
    limit: usize,
) -> BTreeMap<String, String> {
    let generation = server.generation();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut states = BTreeMap::new();
    for (index, thread) in threads.iter().take(limit).enumerate() {
        let state = if index >= 50 || Instant::now() >= deadline {
            "미확인 (서버 조회 한도 50개/전체 3초)".into()
        } else {
            let response = tokio::time::timeout_at(
                deadline.min(Instant::now() + Duration::from_millis(600)),
                server.execute(read_thread(&thread.id, false), Some(generation)),
            )
            .await;
            match response {
                Ok(Ok(value)) => parse(&value, &thread.id),
                Ok(Err(error)) => format!("조회 실패: {error}"),
                Err(_) => "조회 실패: thread/read 시간 제한 초과; 요청 취소".into(),
            }
        };
        states.insert(thread.id.clone(), state.replace(['\r', '\n'], " "));
    }
    if server.generation() != generation {
        for state in states.values_mut() {
            *state = "미확인 (조회 중 서버 세대 변경)".into();
        }
    }
    states
}

fn parse(value: &serde_json::Value, expected: &str) -> String {
    if value
        .pointer("/thread/id")
        .and_then(serde_json::Value::as_str)
        != Some(expected)
    {
        return "조회 실패: thread/read 원본 대화 ID 불일치 또는 누락".into();
    }
    let status = match value
        .pointer("/thread/status/type")
        .and_then(serde_json::Value::as_str)
    {
        Some("active") => match active_state(value) {
            Ok(state) => state,
            Err(error) => return error.into(),
        },
        Some("idle") => "idle (작업 없음)",
        Some("notLoaded") => "notLoaded (서버에 불러오지 않음; 다른 앱 실행 여부 미확인)",
        Some("systemError") => "systemError (서버 오류)",
        _ => return "조회 실패: thread/read 실행 상태 누락 또는 미지원 값".into(),
    };
    format!(
        "{status} · 서버 조회 시점; {}",
        chrono::Utc::now().to_rfc3339()
    )
}

fn active_state(value: &serde_json::Value) -> Result<&'static str, &'static str> {
    let flags = value
        .pointer("/thread/status/activeFlags")
        .and_then(serde_json::Value::as_array)
        .ok_or("조회 실패: activeFlags 누락 또는 배열 아님")?;
    let mut approval = false;
    let mut input = false;
    for flag in flags {
        match flag.as_str() {
            Some("waitingOnApproval") => approval = true,
            Some("waitingOnUserInput") => input = true,
            _ => return Err("조회 실패: 미지원 activeFlags"),
        }
    }
    Ok(match (approval, input) {
        (true, true) => "active (승인 대기 · 사용자 입력 대기)",
        (true, false) => "active (승인 대기)",
        (false, true) => "active (사용자 입력 대기)",
        (false, false) => "active (진행 중)",
    })
}

#[cfg(test)]
mod tests {
    use super::parse;
    use serde_json::json;

    #[test]
    fn only_exact_thread_and_explicit_server_status_can_show_idle() {
        for (id, status, expected) in [
            ("a", "idle", "idle (작업 없음)"),
            ("a", "active", "active (진행 중)"),
            ("a", "notLoaded", "다른 앱 실행 여부 미확인"),
            ("other", "idle", "ID 불일치"),
            ("a", "unknown", "미지원 값"),
        ] {
            let text = parse(
                &json!({"thread":{"id":id,"status":{"type":status,"activeFlags":[]}}}),
                "a",
            );
            assert!(text.contains(expected), "{text}");
        }
        assert!(parse(&json!({"thread":{"id":"a"}}), "a").contains("조회 실패"));
    }

    #[test]
    fn active_waiting_flags_are_not_hidden_as_ordinary_running() {
        for (flags, expected) in [
            (json!(["waitingOnApproval"]), "승인 대기"),
            (json!(["waitingOnUserInput"]), "사용자 입력 대기"),
            (json!(["unknown"]), "미지원 activeFlags"),
            (json!(null), "activeFlags 누락"),
        ] {
            let text = parse(
                &json!({"thread":{"id":"a","status":{"type":"active","activeFlags":flags}}}),
                "a",
            );
            assert!(text.contains(expected), "{text}");
        }
    }
}
