use serde_json::json;

#[test]
fn only_structured_usage_evidence_is_typed() {
    assert!(cdr_app_server::is_usage_limit_error(Some(
        &json!({"type":"usage_limit_reached"})
    )));
    assert!(!cdr_app_server::is_usage_limit_error(Some(
        &json!({"message":"usage limit reached"})
    )));

    let completion = cdr_app_server::outcomes::parse_turn_completion(
        &json!({
            "threadId":"thread-1",
            "turn":{"id":"turn-1","status":"failed","error":{
                "message":"request failed",
                "data":{"type":"usage_limit_reached"}
            }}
        }),
        false,
    )
    .unwrap();
    assert!(completion.usage_limit);
}

#[test]
fn auto_reserve_prefix_is_a_manual_mutation_and_never_a_settings_query() {
    let action =
        cdr_runtime::prefix_plan::plan_prefix("settings thread-1 --auto-reserve on").unwrap();
    assert_eq!(
        action,
        cdr_runtime::prefix_plan::PrefixAction::AutoReserve {
            reference: Some("thread-1".into()),
            enabled: true,
        }
    );
    assert!(
        cdr_runtime::prefix_plan::plan_prefix("settings --auto-reserve on --model gpt-5.6-luna",)
            .is_err()
    );
    assert!(cdr_runtime::prefix_plan::plan_prefix("settings --auto-reserve maybe",).is_err());
}

#[test]
fn usage_start_failure_is_persisted_as_a_hold_not_a_retry() {
    use cdr_runtime::queue_runner::{BackendFailure, BackendFailureKind};

    let failure = BackendFailure::usage_limit("typed server rejection");
    assert_eq!(failure.kind, BackendFailureKind::UsageLimit);
    let held = BackendFailure::persisted(
        format!(
            "{}typed server rejection",
            cdr_store::reserve_policy::HOLD_PREFIX
        ),
        false,
    );
    assert_eq!(held.kind, BackendFailureKind::AutoReserveHeld);
    assert!(!held.ambiguous);
}
