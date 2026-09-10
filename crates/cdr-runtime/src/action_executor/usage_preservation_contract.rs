use super::format_usage;
use chrono::NaiveDate;
use serde_json::json;

#[test]
fn malformed_dates_and_counts_are_labeled_partial_not_zero_or_complete() {
    let text = format_usage(
        2,
        &json!({"rateLimits":{"primary":{"usedPercent":-1,"resetsAt":-1}}}),
        &json!({"dailyUsageBuckets":[
            {"startDate":"2026-09-07","tokens":17},
            {"startDate":"2026-02-30","tokens":50},
            {"startDate":"2026-09-06","tokens":"200"},
            {"startDate":"2026-09-06","tokens":-20}
        ]}),
        NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(),
    );
    assert!(text.contains("period: 2026-09-06 to 2026-09-07 UTC"));
    assert!(text.contains("partial_total_tokens: 17"));
    assert!(text.contains("Warning: malformed usage buckets"));
    assert!(text.contains("primary: used=unavailable window=unavailable resets=unavailable"));
    assert!(!text.lines().any(|line| line.starts_with("total_tokens:")));
}

#[test]
fn unknown_schema_and_empty_array_do_not_claim_no_usage() {
    for usage in [
        json!({"dailyUsageBuckets":[]}),
        json!({"dailyUsageBuckets":{}}),
        json!({"replacementBuckets":[{"tokens":20}]}),
    ] {
        let text = format_usage(
            30,
            &json!({}),
            &usage,
            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(),
        );
        assert!(text.contains("usage data unavailable"));
        assert!(!text.contains("total_tokens:"));
        assert!(text.contains("primary: unavailable"));
    }
}
