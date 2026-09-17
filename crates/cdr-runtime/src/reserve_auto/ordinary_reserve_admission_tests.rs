//! R15-1: policy ordinary is not evidence that the live model is ordinary.
use super::{CommandAction, Fixture, Value, json, reserve_policy};
use crate::queue_runner::{BackendFailureKind, QueueRunnerError, Submission};

async fn existing(ordinary: Value, effort: &str) -> Fixture {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":ordinary,"settings":{
        "model":"gpt-reserve","effort":effort,"serviceTier":"default"
    }}))
    .await;
    reserve_policy::set_mode(&f.db, "thread-b", "manual").unwrap();
    f.controller.set_manual_mode("thread-b", true).unwrap();
    f
}

async fn submit(f: &Fixture) -> Result<Submission, QueueRunnerError> {
    f.queue
        .submit_identified("r15-1-input", "thread-b", 42, 3, None, "r15-1-input")
        .await
}

fn starts(f: &Fixture) -> Vec<Value> {
    f.frames()
        .into_iter()
        .filter(|row| row["event"] == "start_settings")
        .collect()
}

fn policy(f: &Fixture) -> reserve_policy::Policy {
    reserve_policy::get(&f.db, "thread-b").unwrap().unwrap()
}

fn fence(f: &Fixture) -> Option<reserve_policy::usage_fence::Claim> {
    reserve_policy::usage_failure_claim(&f.db, "thread-b").unwrap()
}

fn stage(f: &Fixture) -> reserve_policy::usage_fence::Claim {
    reserve_policy::ensure(&f.db, "thread-b").unwrap();
    reserve_policy::stage_usage_failure(&f.db, "thread-b", "R15-1 prior failure").unwrap();
    fence(f).unwrap()
}

fn assert_held(result: &Result<Submission, QueueRunnerError>) {
    assert!(
        matches!(result, Err(QueueRunnerError::Backend(error))
            if error.kind == BackendFailureKind::AutoReserveHeld),
        "expected held admission, got {result:?}"
    );
}

fn assert_same_failure(
    before: reserve_policy::usage_fence::Claim,
    after: Option<reserve_policy::usage_fence::Claim>,
) {
    let after = after.expect("failed verification must retain the usage fence");
    assert_eq!(after.fence_id, before.fence_id);
    assert_eq!(
        after.failure_policy_revision,
        before.failure_policy_revision
    );
}

#[tokio::test]
async fn manual_high_then_auto_on_with_ordinary_true_aligns_before_first_start() {
    for pending in [false, true] {
        let f = Fixture::new().await;
        f.executor
            .execute(
                CommandAction::Settings {
                    reference: Some("thread-b".into()),
                    model: Some("gpt-reserve".into()),
                    effort: Some("high".into()),
                    speed: None,
                },
                42,
                3,
            )
            .await
            .unwrap();
        f.controller.set_manual_mode("thread-b", true).unwrap();
        f.configure(json!({"ordinary":true})).await;
        if pending {
            stage(&f);
        }
        let before_updates = f.count("thread/settings/update");
        let result = submit(&f).await;
        let observed = starts(&f);
        let after = policy(&f);
        let remaining = fence(&f);
        let updates = f.count("thread/settings/update");
        f.close().await;
        assert!(result.unwrap().turn_id.is_some());
        assert_eq!(updates, before_updates + 1);
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0]["settings"]["effort"], "medium");
        assert_eq!(after.state, "ordinary");
        assert!(after.previous_model.is_none());
        assert!(remaining.is_none());
    }
}

#[tokio::test]
async fn absent_or_null_ordinary_still_validates_existing_reserve() {
    for omitted in [false, true] {
        for pending in [false, true] {
            let f = existing(Value::Null, "high").await;
            f.configure(json!({"omit_ordinary":omitted})).await;
            if pending {
                stage(&f);
            }
            let result = submit(&f).await;
            let observed = starts(&f);
            let updates = f.count("thread/settings/update");
            let after = policy(&f);
            let remaining = fence(&f);
            f.close().await;
            assert!(result.unwrap().turn_id.is_some());
            assert_eq!(updates, 1);
            assert_eq!(observed.len(), 1);
            assert_eq!(observed[0]["settings"]["effort"], "medium");
            assert_eq!(after.state, "ordinary");
            assert!(after.previous_model.is_none());
            assert!(remaining.is_none());
        }
    }
}

#[tokio::test]
async fn low_existing_effort_uses_high_then_medium_without_ordinary_exhaustion() {
    for (supported, expected) in [
        (
            json!([{"reasoningEffort":"low"},{"reasoningEffort":"high"},{"reasoningEffort":"medium"}]),
            "high",
        ),
        (
            json!([{"reasoningEffort":"low"},{"reasoningEffort":"medium"}]),
            "medium",
        ),
    ] {
        let f = existing(json!(true), "low").await;
        f.configure(json!({"reserve_default":null,"reserve_efforts":supported}))
            .await;
        let result = submit(&f).await;
        let observed = starts(&f);
        let updates = f.count("thread/settings/update");
        f.close().await;
        assert!(result.unwrap().turn_id.is_some());
        assert_eq!(updates, 1);
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0]["settings"]["effort"], expected);
    }
}

#[tokio::test]
async fn invalid_initial_reserve_capacity_or_catalog_blocks_without_dispatch() {
    for ordinary in [json!(true), Value::Null] {
        for change in [
            json!({"used":100}),
            json!({"used":null}),
            json!({"spend":true}),
            json!({"reached":"hardLimit"}),
            json!({"reserve_efforts":[{"reasoningEffort":"low"}]}),
        ] {
            let f = existing(ordinary.clone(), "medium").await;
            let before = stage(&f);
            f.configure(change).await;
            let result = submit(&f).await;
            let remaining = fence(&f);
            let updates = f.count("thread/settings/update");
            let count = f.count("turn/start");
            f.close().await;
            assert_held(&result);
            assert_same_failure(before, remaining);
            assert_eq!(updates, 0);
            assert_eq!(count, 0);
        }
    }
}

#[tokio::test]
async fn final_noop_reserve_validation_cannot_clear_fence_on_ordinary_recovery() {
    for ordinary in [json!(true), Value::Null] {
        for change in [json!({"used":100}), json!({"account":"account-b"})] {
            let f = existing(ordinary.clone(), "medium").await;
            let before = stage(&f);
            f.configure(json!({"after_rates":change})).await;
            let result = submit(&f).await;
            let remaining = fence(&f);
            let updates = f.count("thread/settings/update");
            let count = f.count("turn/start");
            f.close().await;
            assert_held(&result);
            assert_eq!(remaining, Some(before));
            assert_eq!(updates, 0);
            assert_eq!(count, 0);
        }
    }
}

#[tokio::test]
async fn final_alignment_failure_retains_fence_and_never_replays_settings() {
    for ordinary in [json!(true), Value::Null] {
        for change in [
            json!({"used":100}),
            json!({"account":"account-b"}),
            json!({"reserve_efforts":[{"reasoningEffort":"high"}]}),
        ] {
            let f = existing(ordinary.clone(), "high").await;
            let before = stage(&f);
            f.configure(json!({"after_settings":change})).await;
            let result = submit(&f).await;
            let after = policy(&f);
            let remaining = fence(&f);
            let repeat = f.controller.prepare_turn("thread-b").await;
            let updates = f.count("thread/settings/update");
            let count = f.count("turn/start");
            f.close().await;
            assert_held(&result);
            assert_eq!(after.state, "unknown");
            assert_same_failure(before, remaining);
            assert!(repeat.is_err());
            assert_eq!(updates, 1);
            assert_eq!(count, 0);
        }
    }
}

#[tokio::test]
async fn valid_noop_reserve_clears_exact_fence_without_revision_or_settings_change() {
    for ordinary in [json!(true), Value::Null] {
        let f = existing(ordinary, "medium").await;
        stage(&f);
        let before = policy(&f);
        let result = submit(&f).await;
        let after = policy(&f);
        let remaining = fence(&f);
        let observed = starts(&f);
        let updates = f.count("thread/settings/update");
        let catalogs = f.count("model/list");
        f.close().await;
        assert!(result.unwrap().turn_id.is_some());
        assert_eq!(after, before);
        assert!(remaining.is_none());
        assert_eq!(updates, 0);
        assert_eq!(catalogs, 2);
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0]["settings"]["effort"], "medium");
    }
}

#[tokio::test]
async fn actual_ordinary_preserves_low_effort_with_optional_quota_absent_or_null() {
    for options in [
        json!({"ordinary":true}),
        json!({"ordinary":null}),
        json!({"omit_ordinary":true}),
    ] {
        let f = Fixture::new().await;
        f.configure(json!({"used":100,"settings":{
            "model":"model-a","effort":"low","serviceTier":"priority"
        }}))
        .await;
        f.configure(options).await;
        let result = submit(&f).await;
        let observed = starts(&f);
        let updates = f.count("thread/settings/update");
        let catalogs = f.count("model/list");
        f.close().await;
        assert!(result.unwrap().turn_id.is_some());
        assert_eq!(updates, 0);
        assert_eq!(catalogs, 0);
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0]["settings"]["model"], "model-a");
        assert_eq!(observed[0]["settings"]["effort"], "low");
    }
}

#[tokio::test]
async fn actual_ordinary_unknown_quota_keeps_pending_failure() {
    for omitted in [false, true] {
        let f = Fixture::new().await;
        f.configure(json!({"ordinary":null,"omit_ordinary":omitted}))
            .await;
        let before = stage(&f);
        let result = submit(&f).await;
        let remaining = fence(&f);
        let count = f.count("turn/start");
        let updates = f.count("thread/settings/update");
        f.close().await;
        assert_held(&result);
        assert_eq!(remaining, Some(before));
        assert_eq!(count, 0);
        assert_eq!(updates, 0);
    }
}

#[tokio::test]
async fn actual_ordinary_recovery_resolves_exact_failure_without_reserve_validation() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"used":100})).await;
    stage(&f);
    let result = submit(&f).await;
    let remaining = fence(&f);
    let observed = starts(&f);
    let updates = f.count("thread/settings/update");
    f.close().await;
    assert!(result.unwrap().turn_id.is_some());
    assert!(remaining.is_none());
    assert_eq!(updates, 0);
    assert_eq!(observed[0]["settings"]["model"], "model-a");
}

#[tokio::test]
async fn manual_and_off_do_not_adopt_automatic_effort_policy() {
    for mode in ["manual", "off"] {
        let f = existing(Value::Null, "low").await;
        reserve_policy::set_mode(&f.db, "thread-b", mode).unwrap();
        f.configure(json!({"used":100})).await;
        let result = submit(&f).await;
        let observed = starts(&f);
        let updates = f.count("thread/settings/update");
        f.close().await;
        assert!(result.unwrap().turn_id.is_some());
        assert_eq!(updates, 0);
        assert_eq!(observed[0]["settings"]["effort"], "low");
    }
}

#[tokio::test]
async fn legacy_ownerless_fence_cannot_be_consumed_by_existing_reserve() {
    for effort in ["medium", "high"] {
        let f = existing(json!(true), effort).await;
        stage(&f);
        rusqlite::Connection::open(&f.db).unwrap().execute(
            "UPDATE codex_reserve_policy SET usage_failure_revision=NULL WHERE thread_id='thread-b'", [],
        ).unwrap();
        let before = fence(&f).unwrap();
        let result = submit(&f).await;
        let remaining = fence(&f);
        let count = f.count("turn/start");
        f.close().await;
        assert_held(&result);
        assert_same_failure(before, remaining);
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn new_transition_still_requires_ordinary_exhaustion_after_settings_apply() {
    let f = Fixture::new().await;
    f.configure(json!({"after_settings":{"ordinary":true}}))
        .await;
    let before = stage(&f);
    let result = submit(&f).await;
    let remaining = fence(&f);
    let after = policy(&f);
    let count = f.count("turn/start");
    let updates = f.count("thread/settings/update");
    f.close().await;
    assert_held(&result);
    assert_same_failure(before, remaining);
    assert_eq!(after.state, "unknown");
    assert_eq!(count, 0);
    assert_eq!(updates, 1);
}
