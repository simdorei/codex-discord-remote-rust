//! User-facing context measurements, not cumulative account usage.
use cdr_codex_state::ContextUsage;
use chrono::{DateTime, Utc};

#[must_use]
pub fn usage_lines(
    usage: Option<&ContextUsage>,
    cumulative: Option<i64>,
    now: DateTime<Utc>,
) -> String {
    let cumulative = cumulative
        .and_then(|v| u64::try_from(v).ok())
        .map_or_else(|| "미확인".into(), |v| v.to_string());
    let mut lines = vec![format!(
        "used: {cumulative} · 누적 사용량, 현재 context 아님"
    )];
    let Some(usage) = usage else {
        lines.push(
            "last_input: 미확인 · 확인된 측정값 없음\npeak_input: 미확인\nwindow: 미확인".into(),
        );
        return lines.join("\n");
    };
    lines.push(format!(
        "last_input: {} · 마지막 관측, 현재 실시간 값 아님",
        usage.last_input_tokens
    ));
    lines.push(format!(
        "peak_input: {} · 기록 내 최대",
        usage.peak_input_tokens
    ));
    lines.push(format!("last_total: {}", optional(usage.last_total_tokens)));
    lines.push(match usage.model_context_window.filter(|v| *v > 0) {
        Some(window) => {
            let hundredths = u128::from(usage.last_input_tokens) * 10_000 / u128::from(window);
            format!(
                "window: {window} · 마지막 입력 비율 {}.{:02}%",
                hundredths / 100,
                hundredths % 100
            )
        }
        None => "window: 미확인 · 비율 계산 불가".into(),
    });
    lines.push(observation_age(usage.observed_at.as_deref(), now));
    lines.push(format!(
        "압축 추정: {} · 입력 감소로 추정, 실제 압축 횟수 확정 아님",
        usage.inferred_compactions
    ));
    if let Some((before, after)) = usage.last_compaction {
        lines.push(format!("마지막 추정 감소: {before} → {after}"));
    }
    lines.join("\n")
}

fn optional(value: Option<u64>) -> String {
    value.map_or_else(|| "미확인".into(), |v| v.to_string())
}

fn observation_age(value: Option<&str>, now: DateTime<Utc>) -> String {
    let Some(observed) = value.and_then(|v| DateTime::parse_from_rfc3339(v).ok()) else {
        return "관측 시각 미확인 · 최신 여부 판단 불가".into();
    };
    let seconds = now.signed_duration_since(observed).num_seconds();
    if seconds < 0 {
        format!(
            "관측 시각이 미래: {} · 최신 여부 판단 불가",
            observed.to_rfc3339()
        )
    } else {
        format!("관측: {} · {seconds}초 전", observed.to_rfc3339())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_observation_is_not_cumulative_or_current_measurement() {
        let usage = ContextUsage {
            last_input_tokens: 80_000,
            peak_input_tokens: 150_000,
            last_total_tokens: Some(85_000),
            model_context_window: Some(200_000),
            inferred_compactions: 1,
            last_compaction: Some((150_000, 80_000)),
            observed_at: Some("2026-09-07T10:00:00Z".into()),
        };
        let now = "2026-09-07T11:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let text = usage_lines(Some(&usage), Some(100_000_000), now);
        for expected in [
            "last_input: 80000",
            "peak_input: 150000",
            "window: 200000",
            "40.00%",
            "used: 100000000",
            "3600초 전",
            "현재 실시간 값 아님",
            "압축 추정: 1",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
    }

    #[test]
    fn absent_measurement_is_unknown_and_bad_clock_is_not_fresh() {
        let now = "2026-09-07T11:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let text = usage_lines(None, Some(-1), now);
        assert!(text.contains("last_input: 미확인"));
        assert!(text.contains("used: 미확인"));
        assert!(!text.contains("0%"));
        let mut usage = ContextUsage {
            last_input_tokens: 0,
            peak_input_tokens: 0,
            last_total_tokens: None,
            model_context_window: None,
            inferred_compactions: 0,
            last_compaction: None,
            observed_at: Some("2026-09-07T12:00:00Z".into()),
        };
        assert!(usage_lines(Some(&usage), Some(0), now).contains("관측 시각이 미래"));
        usage.observed_at = Some("bad timestamp".into());
        let text = usage_lines(Some(&usage), Some(0), now);
        assert!(text.contains("관측 시각 미확인"));
        assert!(text.contains("window: 미확인"));
        assert!(text.contains("last_input: 0"));
    }
}
