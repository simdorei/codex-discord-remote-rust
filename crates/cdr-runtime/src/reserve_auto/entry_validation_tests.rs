//! Revision 16: an ordinary policy is not proof of ordinary live settings.
use super::{Fixture, json, reserve_policy};
use crate::queue_runner::{BackendFailureKind, QueueRunnerError, Submission};
use serde_json::Value;
use std::time::Duration;

async fn existing(ordinary: Value, effort: &str) -> Fixture {
    let f = Fixture::new().await;
    reserve_policy::ensure(&f.db, "thread-b").unwrap();
    reserve_policy::set_mode(&f.db, "thread-b", "manual").unwrap();
    f.configure(json!({"ordinary":ordinary,
        "settings":{"model":"gpt-reserve","effort":effort,"serviceTier":"default"}}))
        .await;
    reserve_policy::set_mode(&f.db, "thread-b", "on").unwrap();
    f
}

async fn submit(f: &Fixture) -> Result<Submission, QueueRunnerError> {
    f.queue
        .submit_identified("r16-input", "thread-b", 42, 3, None, "r16-input")
        .await
}

fn assert_held(result: &Result<Submission, QueueRunnerError>) {
    assert!(
        matches!(result, Err(QueueRunnerError::Backend(error))
        if error.kind == BackendFailureKind::AutoReserveHeld),
        "expected hold: {result:?}"
    );
}

fn claim(f: &Fixture) -> Option<reserve_policy::usage_fence::Claim> {
    reserve_policy::usage_failure_claim(&f.db, "thread-b").unwrap()
}

fn stage(f: &Fixture) {
    reserve_policy::stage_usage_failure(&f.db, "thread-b", "revision16 prior usage failure")
        .unwrap();
}

fn efforts(values: &[&str]) -> Value {
    json!(
        values
            .iter()
            .map(|value| json!({"reasoningEffort":value}))
            .collect::<Vec<_>>()
    )
}

#[tokio::test]
async fn ordinary_true_existing_reserve_aligns_default_and_never_guesses_restore() {
    let f = existing(json!(true), "high").await;
    let result = submit(&f).await;
    let after = reserve_policy::get(&f.db, "thread-b").unwrap().unwrap();
    let starts: Vec<_> = f
        .frames()
        .into_iter()
        .filter(|row| row["event"] == "start_settings")
        .collect();
    if let Ok(job) = &result
        && let Some(turn) = &job.turn_id
    {
        f.finish(turn, false).await;
    }
    let again = f.controller.prepare_turn("thread-b").await;
    let updates = f.count("thread/settings/update");
    let live = f.rpc("thread/resume", json!({})).await;
    f.close().await;
    assert!(result.unwrap().turn_id.is_some());
    again.unwrap();
    assert_eq!(updates, 1);
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["settings"]["effort"], "medium");
    assert_eq!(live["model"], "gpt-reserve");
    assert_eq!(live["reasoningEffort"], "medium");
    assert_eq!(after.state, "ordinary");
    assert!(after.previous_model.is_none());
    assert!(after.previous_effort.is_none());
    assert!(after.previous_tier.is_none());
}

#[tokio::test]
async fn ordinary_null_existing_reserve_aligns_low_before_first_start() {
    let f = existing(Value::Null, "low").await;
    let result = submit(&f).await;
    let updates = f.count("thread/settings/update");
    let starts: Vec<_> = f
        .frames()
        .into_iter()
        .filter(|row| row["event"] == "start_settings")
        .collect();
    f.close().await;
    assert!(result.unwrap().turn_id.is_some());
    assert_eq!(updates, 1);
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["settings"]["effort"], "medium");
}

#[tokio::test]
async fn existing_reserve_noop_checks_capacity_catalog_and_resolves_only_verified_claim() {
    for ordinary in [json!(true), Value::Null] {
        for pending in [false, true] {
            let f = existing(ordinary.clone(), "medium").await;
            if pending {
                stage(&f);
            }
            let result = submit(&f).await;
            let updates = f.count("thread/settings/update");
            let rates = f.count("account/rateLimits/read");
            let models = f.count("model/list");
            let remaining = claim(&f);
            f.close().await;
            assert!(result.unwrap().turn_id.is_some());
            assert_eq!(updates, 0);
            assert!(rates >= 2, "missing final capacity/account read: {rates}");
            assert_eq!(models, 2);
            assert!(remaining.is_none());
        }
    }
}

#[tokio::test]
async fn existing_reserve_alignment_uses_high_then_medium_without_low_fallback() {
    for ordinary in [json!(true), Value::Null] {
        for (supported, expected) in [
            (efforts(&["low", "medium", "high"]), "high"),
            (efforts(&["low", "medium"]), "medium"),
        ] {
            let f = existing(ordinary.clone(), "low").await;
            stage(&f);
            f.configure(json!({"reserve_default":null,"reserve_efforts":supported}))
                .await;
            let result = submit(&f).await;
            let remaining = claim(&f);
            let starts: Vec<_> = f
                .frames()
                .into_iter()
                .filter(|row| row["event"] == "start_settings")
                .collect();
            let updates = f.count("thread/settings/update");
            f.close().await;
            assert!(result.unwrap().turn_id.is_some());
            assert!(remaining.is_none());
            assert_eq!(updates, 1);
            assert_eq!(starts.len(), 1);
            assert_eq!(starts[0]["settings"]["effort"], expected);
        }
    }
}

#[tokio::test]
async fn existing_reserve_without_supported_medium_or_higher_preserves_fence() {
    for ordinary in [json!(true), Value::Null] {
        let f = existing(ordinary, "low").await;
        stage(&f);
        let before = claim(&f);
        f.configure(json!({"reserve_default":"low","reserve_efforts":efforts(&["low"])}))
            .await;
        let result = submit(&f).await;
        let remaining = claim(&f);
        let updates = f.count("thread/settings/update");
        let starts = f.count("turn/start");
        f.close().await;
        assert_held(&result);
        assert_eq!(remaining, before);
        assert_eq!(updates, 0);
        assert_eq!(starts, 0);
    }
}

#[tokio::test]
async fn existing_reserve_initial_capacity_failure_never_clears_fence_or_starts() {
    for ordinary in [json!(true), Value::Null] {
        for change in [
            json!({"used":100}),
            json!({"spend":true}),
            json!({"normal":"missing-model"}),
        ] {
            let f = existing(ordinary.clone(), "medium").await;
            stage(&f);
            let before = claim(&f);
            f.configure(change).await;
            let result = submit(&f).await;
            let remaining = claim(&f);
            let updates = f.count("thread/settings/update");
            let starts = f.count("turn/start");
            f.close().await;
            assert_held(&result);
            assert_eq!(remaining, before);
            assert_eq!(updates, 0);
            assert_eq!(starts, 0);
        }
    }
}

#[tokio::test]
async fn existing_reserve_final_account_capacity_or_effort_change_preserves_exact_fence() {
    for ordinary in [json!(true), Value::Null] {
        for change in [
            json!({"used":100}),
            json!({"account":"account-b"}),
            json!({"normal":"model-b"}),
            json!({"spend":true}),
        ] {
            let f = existing(ordinary.clone(), "high").await;
            stage(&f);
            let before = claim(&f);
            f.configure(json!({"reserve_default":"high","after_rates":change}))
                .await;
            let result = submit(&f).await;
            let remaining = claim(&f);
            let starts = f.count("turn/start");
            let updates = f.count("thread/settings/update");
            f.close().await;
            assert_held(&result);
            assert_eq!(remaining, before);
            assert_eq!(starts, 0);
            assert_eq!(updates, 0);
        }
    }
}

#[tokio::test]
async fn existing_reserve_alignment_final_failure_is_held_and_never_replayed() {
    for ordinary in [json!(true), Value::Null] {
        let f = existing(ordinary, "low").await;
        stage(&f);
        f.configure(json!({"reserve_default":"high","after_settings":{"normal":"model-b"}}))
            .await;
        let result = submit(&f).await;
        let after = reserve_policy::get(&f.db, "thread-b").unwrap().unwrap();
        let remaining = claim(&f);
        let repeat = f.controller.prepare_turn("thread-b").await;
        let updates = f.count("thread/settings/update");
        let starts = f.count("turn/start");
        f.close().await;
        assert_held(&result);
        assert_eq!(after.state, "unknown");
        assert!(remaining.is_some());
        assert!(repeat.is_err());
        assert_eq!(updates, 1);
        assert_eq!(starts, 0);
    }
}

#[tokio::test]
async fn ordinary_model_optional_quota_field_is_allowed_only_without_failure_fence() {
    for pending in [false, true] {
        let f = Fixture::new().await;
        reserve_policy::ensure(&f.db, "thread-b").unwrap();
        f.configure(json!({"ordinary":null})).await;
        if pending {
            stage(&f);
        }
        let before = claim(&f);
        let result = submit(&f).await;
        let remaining = claim(&f);
        let models = f.count("model/list");
        let updates = f.count("thread/settings/update");
        let starts = f.count("turn/start");
        f.close().await;
        if pending {
            assert_held(&result);
            assert_eq!(remaining, before);
            assert_eq!(starts, 0);
        } else {
            assert!(result.unwrap().turn_id.is_some());
            assert_eq!(starts, 1);
        }
        assert_eq!(models, 0);
        assert_eq!(updates, 0);
    }
}

#[tokio::test]
async fn ordinary_model_known_recovery_resolves_claim_without_reserve_changes() {
    let f = Fixture::new().await;
    reserve_policy::ensure(&f.db, "thread-b").unwrap();
    stage(&f);
    f.configure(json!({"ordinary":true,"used":100})).await;
    let result = submit(&f).await;
    let remaining = claim(&f);
    let updates = f.count("thread/settings/update");
    let models = f.count("model/list");
    f.close().await;
    assert!(result.unwrap().turn_id.is_some());
    assert!(remaining.is_none());
    assert_eq!(updates, 0);
    assert_eq!(models, 0);
}

#[tokio::test]
async fn manual_and_off_do_not_align_or_validate_automatic_reserve_effort() {
    for mode in ["manual", "off"] {
        let f = existing(json!(true), "low").await;
        reserve_policy::set_mode(&f.db, "thread-b", mode).unwrap();
        f.configure(json!({"used":100,"reserve_efforts":efforts(&["low"])}))
            .await;
        let result = submit(&f).await;
        let updates = f.count("thread/settings/update");
        let rates = f.count("account/rateLimits/read");
        let starts: Vec<_> = f
            .frames()
            .into_iter()
            .filter(|row| row["event"] == "start_settings")
            .collect();
        f.close().await;
        assert!(result.unwrap().turn_id.is_some());
        assert_eq!(updates, 0);
        assert_eq!(rates, 0);
        assert_eq!(starts[0]["settings"]["effort"], "low");
    }
}

#[tokio::test]
async fn ordinary_true_does_not_bypass_exact_thread_observation() {
    let f = existing(json!(true), "medium").await;
    stage(&f);
    let before = claim(&f);
    f.configure(json!({"wrong_thread":true})).await;
    let result = submit(&f).await;
    let remaining = claim(&f);
    let starts = f.count("turn/start");
    f.close().await;
    assert!(result.is_err());
    assert_eq!(remaining, before);
    assert_eq!(starts, 0);
}

#[tokio::test]
async fn ordinary_existing_reserve_noop_cannot_clear_a_newer_fence_or_override_policy() {
    for override_policy in [false, true] {
        let f = existing(json!(true), "medium").await;
        stage(&f);
        f.configure(json!({"gate":"model/list"})).await;
        let controller = f.controller.clone();
        let task = tokio::spawn(async move { controller.prepare_turn("thread-b").await });
        for _ in 0..300 {
            if f.temp.path().join("gate-ready").exists() || task.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let reached = f.temp.path().join("gate-ready").exists();
        if override_policy {
            reserve_policy::set_mode(&f.db, "thread-b", "off").unwrap();
        } else {
            stage(&f);
        }
        let latest = claim(&f);
        std::fs::write(f.temp.path().join("gate-release"), "release").unwrap();
        let result = task.await.unwrap();
        let remaining = claim(&f);
        let updates = f.count("thread/settings/update");
        let after = reserve_policy::get(&f.db, "thread-b").unwrap().unwrap();
        f.close().await;
        assert!(
            reached,
            "ordinary entry skipped Reserve verification entirely"
        );
        assert!(result.is_err());
        assert_eq!(remaining, latest);
        assert_eq!(updates, 0);
        if override_policy {
            assert_eq!(after.mode, "off");
        }
    }
}
