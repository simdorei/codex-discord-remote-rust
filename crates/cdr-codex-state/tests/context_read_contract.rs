use cdr_codex_state::{ContextReadBudget, read_context_usage};

const META: &str = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"original\"}}\n";
const USAGE: &str = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":80000},\"total_token_usage\":{\"input_tokens\":100000000}}}}\n";

#[test]
fn complete_original_session_returns_last_input_not_cumulative_usage() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("session.jsonl");
    std::fs::write(&path, format!("{META}{USAGE}")).unwrap();
    let result = read_context_usage(&path, "original", ContextReadBudget::default())
        .unwrap()
        .unwrap();
    assert_eq!(result.last_input_tokens, 80_000);
    assert_eq!(result.peak_input_tokens, 80_000);
    assert_eq!(result.model_context_window, None);
}

#[test]
fn wrong_identity_truncated_or_malformed_records_never_return_old_usage() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("session.jsonl");
    for contents in [
        format!("{}{}", META.replace("original", "other"), USAGE),
        format!("{META}{USAGE}{{\"type\":"),
        format!("{META}{USAGE}not-json\n"),
        USAGE.to_owned(),
        format!("{META}{USAGE}{}", META.replace("original", "other")),
    ] {
        std::fs::write(&path, contents).unwrap();
        assert!(read_context_usage(&path, "original", ContextReadBudget::default()).is_err());
    }
}

#[test]
fn no_token_observation_is_unknown_and_limits_fail_explicitly() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("session.jsonl");
    std::fs::write(&path, META).unwrap();
    assert!(
        read_context_usage(&path, "original", ContextReadBudget::default())
            .unwrap()
            .is_none()
    );
    for budget in [
        ContextReadBudget {
            max_bytes: 1,
            ..Default::default()
        },
        ContextReadBudget {
            max_line_bytes: 8,
            ..Default::default()
        },
        ContextReadBudget {
            max_duration: std::time::Duration::ZERO,
            ..Default::default()
        },
    ] {
        assert!(read_context_usage(&path, "original", budget).is_err());
    }
    assert!(
        read_context_usage(
            &temp.path().join("missing"),
            "original",
            ContextReadBudget::default()
        )
        .is_err()
    );
}
