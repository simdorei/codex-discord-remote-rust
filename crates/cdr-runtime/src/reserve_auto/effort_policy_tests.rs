//! Revision 15: default/high/medium selection and exact final verification.
use super::*;
use crate::queue_runner::{BackendFailureKind, QueueRunnerError};

fn supported(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|effort| json!({"reasoningEffort":effort}))
            .collect(),
    )
}

fn policy(f: &Fixture) -> reserve_policy::Policy {
    reserve_policy::get(&f.db, "thread-b").unwrap().unwrap()
}

fn notices(f: &Fixture) -> i64 {
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM codex_reserve_transition_notices",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

async fn submit_result(f: &Fixture) -> Result<crate::queue_runner::Submission, QueueRunnerError> {
    f.queue
        .submit_identified("r15-input", "thread-b", 42, 3, None, "r15-input")
        .await
}

fn assert_held(result: &Result<crate::queue_runner::Submission, QueueRunnerError>) {
    assert!(
        matches!(result, Err(QueueRunnerError::Backend(error))
        if error.kind == BackendFailureKind::AutoReserveHeld),
        "expected held admission: {result:?}"
    );
}

#[tokio::test]
async fn r15_default_is_selected_instead_of_previous_effort() {
    for default in ["medium", "high", "xhigh"] {
        let f = Fixture::new().await;
        f.configure(json!({"reserve_default":default})).await;
        let result = submit_result(&f).await;
        let starts: Vec<_> = f
            .frames()
            .into_iter()
            .filter(|row| row["event"] == "start_settings")
            .collect();
        let count = f.count("thread/settings/update");
        f.close().await;
        assert!(result.unwrap().turn_id.is_some());
        assert_eq!(count, 1);
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["settings"]["model"], "gpt-reserve");
        assert_eq!(starts[0]["settings"]["effort"], default);
    }
}

#[tokio::test]
async fn r15_absent_low_or_unsupported_default_uses_high_then_medium() {
    for default in [Value::Null, json!("low"), json!("unsupported"), json!(7)] {
        for (values, expected) in [
            (vec!["low", "medium", "high"], "high"),
            (vec!["low", "medium"], "medium"),
        ] {
            let f = Fixture::new().await;
            f.configure(
                json!({"reserve_default":default,"reserve_efforts":supported(&values),
                "settings":{"model":"model-a","effort":"low","serviceTier":"priority"}}),
            )
            .await;
            let result = submit_result(&f).await;
            let starts: Vec<_> = f
                .frames()
                .into_iter()
                .filter(|row| row["event"] == "start_settings")
                .collect();
            f.close().await;
            assert!(result.unwrap().turn_id.is_some());
            assert_eq!(starts.len(), 1);
            assert_eq!(starts[0]["settings"]["effort"], expected);
        }
    }
}

#[tokio::test]
async fn r15_below_medium_and_missing_support_never_send_settings_or_start() {
    for values in [
        supported(&["none", "minimal", "low"]),
        json!([]),
        Value::Null,
    ] {
        let f = Fixture::new().await;
        f.configure(json!({"reserve_default":"low","reserve_efforts":values}))
            .await;
        let result = submit_result(&f).await;
        let updates = f.count("thread/settings/update");
        let starts = f.count("turn/start");
        f.close().await;
        assert_held(&result);
        assert_eq!(updates, 0);
        assert_eq!(starts, 0);
    }
}

#[tokio::test]
async fn r15_existing_episode_realigns_effort_and_preserves_restore_snapshot() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    let before = policy(&f);
    f.configure(json!({"reserve_default":"xhigh"})).await;
    reserve_policy::stage_usage_failure(&f.db, "thread-b", "prior failure").unwrap();
    let result = submit_result(&f).await;
    let after = policy(&f);
    let frames = f.frames();
    let updates = f.count("thread/settings/update");
    let unresolved = reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap();
    if let Ok(job) = &result
        && let Some(turn) = &job.turn_id
    {
        f.finish(turn, false).await;
    }
    f.configure(json!({"ordinary":true})).await;
    let restore = f.controller.prepare_turn("thread-b").await;
    let restored = f.rpc("thread/resume", json!({})).await;
    f.close().await;
    assert!(result.unwrap().turn_id.is_some());
    assert_eq!(updates, 2);
    assert_eq!(after.state, "reserve");
    assert_eq!(after.applied_effort.as_deref(), Some("xhigh"));
    assert_eq!(after.previous_model, before.previous_model);
    assert_eq!(after.previous_effort, before.previous_effort);
    assert_eq!(
        after.previous_effort_present,
        before.previous_effort_present
    );
    assert_eq!(after.previous_tier, before.previous_tier);
    assert!(!unresolved);
    assert_eq!(
        frames
            .iter()
            .find(|r| r["event"] == "start_settings")
            .unwrap()["settings"]["effort"],
        "xhigh"
    );
    restore.unwrap();
    assert_eq!(restored["model"], "model-a");
    assert_eq!(restored["reasoningEffort"], "high");
    assert_eq!(restored["serviceTier"], "priority");
}

#[tokio::test]
async fn r15_already_reserve_without_episode_aligns_without_inventing_restore() {
    let f = Fixture::new().await;
    reserve_policy::ensure(&f.db, "thread-b").unwrap();
    reserve_policy::stage_usage_failure(&f.db, "thread-b", "prior failure").unwrap();
    f.configure(json!({"settings":{"model":"gpt-reserve","effort":"low","serviceTier":"default"}}))
        .await;
    let result = submit_result(&f).await;
    let after = policy(&f);
    let updates = f.count("thread/settings/update");
    let unresolved = reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap();
    let starts: Vec<_> = f
        .frames()
        .into_iter()
        .filter(|row| row["event"] == "start_settings")
        .collect();
    f.close().await;
    assert!(result.unwrap().turn_id.is_some());
    assert_eq!(updates, 1);
    assert_eq!(after.state, "ordinary");
    assert!(after.previous_model.is_none());
    assert!(!unresolved);
    assert_eq!(starts[0]["settings"]["effort"], "medium");
}

async fn final_reuse_case(ordinary_policy: bool, fence: bool, change: Value) {
    let f = Fixture::new().await;
    f.configure(json!({"reserve_default":"high"})).await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    if ordinary_policy {
        reserve_policy::set_mode(&f.db, "thread-b", "manual").unwrap();
        reserve_policy::set_mode(&f.db, "thread-b", "on").unwrap();
    }
    if fence {
        reserve_policy::stage_usage_failure(&f.db, "thread-b", "prior failure").unwrap();
    }
    let claim = reserve_policy::usage_failure_claim(&f.db, "thread-b").unwrap();
    let notices_before = notices(&f);
    f.configure(json!({"after_rates":change})).await;
    let result = submit_result(&f).await;
    let remaining = reserve_policy::usage_failure_claim(&f.db, "thread-b").unwrap();
    let updates = f.count("thread/settings/update");
    let starts = f.count("turn/start");
    let notices_after = notices(&f);
    f.close().await;
    assert_held(&result);
    assert_eq!(updates, 1, "no change was needed before final validation");
    assert_eq!(starts, 0);
    assert_eq!(remaining, claim);
    assert_eq!(notices_before, notices_after);
}

#[tokio::test]
async fn r15_final_effort_validation_covers_existing_and_ordinary_reserve_with_fences() {
    for ordinary_policy in [false, true] {
        for fence in [false, true] {
            final_reuse_case(ordinary_policy, fence, json!({"normal":"model-b"})).await;
        }
    }
}

#[tokio::test]
async fn r15_final_capacity_validation_covers_reserve_reuse_without_settings_rpc() {
    for change in [
        json!({"used":100}),
        json!({"spend":true}),
        json!({"reached":"hardLimit"}),
        json!({"used":null}),
        json!({"normal":"missing-model"}),
    ] {
        final_reuse_case(false, true, change).await;
    }
}

#[tokio::test]
async fn r15_realign_final_mismatch_is_unknown_and_does_not_replay() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    reserve_policy::stage_usage_failure(&f.db, "thread-b", "prior failure").unwrap();
    f.configure(json!({"reserve_default":"high","after_settings":{"normal":"model-b"}}))
        .await;
    let before_notices = notices(&f);
    let result = submit_result(&f).await;
    let after = policy(&f);
    let remaining = reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap();
    let repeat = f.controller.prepare_turn("thread-b").await;
    let updates = f.count("thread/settings/update");
    let starts = f.count("turn/start");
    let after_notices = notices(&f);
    f.close().await;
    assert_held(&result);
    assert_eq!(after.state, "unknown");
    assert!(remaining);
    assert!(repeat.is_err());
    assert_eq!(updates, 2);
    assert_eq!(starts, 0);
    assert_eq!(before_notices, after_notices);
}

#[path = "effort_boundary_tests.rs"]
mod boundaries;
