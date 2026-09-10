use chrono::{DateTime, Days, NaiveDate, Utc};
use serde_json::Value;

#[cfg(test)]
#[path = "usage_preservation_contract.rs"]
mod preservation_contract;

pub(super) fn format_usage(days: u32, rates: &Value, usage: &Value, today: NaiveDate) -> String {
    let days = days.clamp(1, 30);
    let first = today
        .checked_sub_days(Days::new(u64::from(days - 1)))
        .unwrap_or(today);
    let limits = &rates["rateLimits"];
    let mut lines = vec![
        format!("Codex usage ({days}d live)"),
        format!("period: {first} to {today} UTC"),
        format!(
            "plan: {}",
            limits["planType"].as_str().unwrap_or("unavailable")
        ),
        window("primary", &limits["primary"]),
        window("secondary", &limits["secondary"]),
    ];
    if let Some(credits) = limits.get("credits").filter(|v| v.is_object()) {
        lines.push(format!(
            "credits: balance={} unlimited={}",
            scalar(&credits["balance"]),
            scalar(&credits["unlimited"])
        ));
    }
    lines.push(String::from("\nDaily token usage"));
    let mut rows = Vec::new();
    let mut invalid = false;
    if let Some(buckets) = usage["dailyUsageBuckets"].as_array() {
        for bucket in buckets {
            let date = bucket["startDate"]
                .as_str()
                .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
            match (date, bucket["tokens"].as_u64()) {
                (Some(date), Some(tokens)) if date >= first && date <= today => {
                    rows.push((date, tokens));
                }
                (Some(_), Some(_)) => {}
                _ => invalid = true,
            }
        }
    }
    rows.sort_unstable();
    if rows.is_empty() {
        lines
            .push("usage data unavailable: no valid usage buckets returned for this period".into());
    } else {
        let total: u128 = rows.iter().map(|(_, tokens)| u128::from(*tokens)).sum();
        for (date, tokens) in rows {
            lines.push(format!("{date}: {tokens}"));
        }
        lines.push(format!(
            "{}: {total}",
            if invalid {
                "partial_total_tokens"
            } else {
                "total_tokens"
            }
        ));
    }
    if invalid {
        lines.push("Warning: malformed usage buckets; totals may be incomplete.".into());
    }
    lines.push(String::from("\nAccount summary"));
    if let Some(summary) = usage.get("summary").filter(|v| v.is_object()) {
        for (key, label) in [
            ("currentStreakDays", "current_streak_days"),
            ("longestStreakDays", "longest_streak_days"),
            ("lifetimeTokens", "lifetime_tokens"),
            ("peakDailyTokens", "peak_daily_tokens"),
            ("longestRunningTurnSec", "longest_running_turn_sec"),
        ] {
            lines.push(format!("{label}: {}", scalar(&summary[key])));
        }
    } else {
        lines.push("unavailable".into());
    }
    lines.join("\n")
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(_) | Value::Bool(_) => value.to_string(),
        _ => "unavailable".into(),
    }
}

fn window(label: &str, value: &Value) -> String {
    if !value.is_object() {
        return format!("{label}: unavailable");
    }
    let percent = value["usedPercent"]
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map_or_else(|| "unavailable".into(), |v| format!("{v}%"));
    let duration = match value["windowDurationMins"].as_u64() {
        Some(n) if n > 0 && n % 1440 == 0 => format!("{}d", n / 1440),
        Some(n) if n > 0 && n % 60 == 0 => format!("{}h", n / 60),
        Some(n) if n > 0 => format!("{n}m"),
        _ => "unavailable".into(),
    };
    let reset = value["resetsAt"]
        .as_i64()
        .filter(|v| *v > 0)
        .and_then(|v| DateTime::<Utc>::from_timestamp(v, 0))
        .map_or_else(
            || "unavailable".into(),
            |v| v.format("%Y-%m-%d %H:%M UTC").to_string(),
        );
    format!("{label}: used={percent} window={duration} resets={reset}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_is_readable_and_filters_the_requested_utc_days() {
        let rates = json!({"rateLimits":{"planType":"pro", "primary":{
            "usedPercent":25, "windowDurationMins":300, "resetsAt":0
        }}});
        let usage = json!({"dailyUsageBuckets":[
            {"startDate":"2026-09-06","tokens":2000},
            {"startDate":"2026-09-05","tokens":1000},
            {"startDate":"2026-09-04","tokens":900_000},
            {"startDate":"2026-09-07","tokens":800_000}
        ],"summary":{"lifetimeTokens":5_000_000}});
        let text = format_usage(
            2,
            &rates,
            &usage,
            NaiveDate::from_ymd_opt(2026, 9, 6).unwrap(),
        );
        assert!(text.contains("primary: used=25% window=5h"), "{text}");
        assert!(text.contains("total_tokens: 3000"), "{text}");
        assert!(
            !text.contains("900000") && !text.contains("800000"),
            "{text}"
        );
        assert!(text.contains("lifetime_tokens: 5000000"), "{text}");
        assert!(!text.contains("dailyUsageBuckets"), "{text}");
    }

    #[test]
    fn missing_usage_is_not_reported_as_zero_consumption() {
        let text = format_usage(
            7,
            &json!({}),
            &json!({}),
            NaiveDate::from_ymd_opt(2026, 9, 6).unwrap(),
        );
        assert!(text.contains("usage data unavailable"), "{text}");
        assert!(!text.contains("total_tokens: 0"), "{text}");
    }
}
