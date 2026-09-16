//! Revision 14: the final account response must authorize the settings just applied.
use super::*;
use crate::queue_runner::{BackendFailureKind, QueueRunnerError};

async fn assert_final_rates_hold(change: Value) {
    for with_failure_fence in [false, true] {
        let f = Fixture::new().await;
        reserve_policy::ensure(&f.db, "thread-b").unwrap();
        if with_failure_fence {
            reserve_policy::stage_usage_failure(&f.db, "thread-b", "prior typed usage failure")
                .unwrap();
        }
        f.configure(
            json!({"ordinary":false,"used":1,"reserve_default":"high","after_settings":change}),
        )
        .await;
        // prepare_turn fails before an attempt is admitted, not as a start warning.
        let result = f
            .queue
            .submit_identified(
                "r14-must-not-start",
                "thread-b",
                42,
                3,
                None,
                "r14-must-not-start",
            )
            .await;
        assert!(
            matches!(result, Err(QueueRunnerError::Backend(ref error)) if error.kind == BackendFailureKind::AutoReserveHeld),
            "R13-3: final Reserve response must reject admission: {result:?}"
        );
        let policy = reserve_policy::get(&f.db, "thread-b").unwrap().unwrap();
        assert_eq!(
            policy.state, "unknown",
            "applied settings must not become a successful episode"
        );
        assert_eq!(
            reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap(),
            with_failure_fence
        );
        assert_eq!(f.count("thread/settings/update"), 1);
        assert_eq!(f.count("turn/start"), 0);
        let success_notices: i64 = rusqlite::Connection::open(&f.db)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM codex_reserve_transition_notices",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(success_notices, 0);
        // A repeat tick must not resend the settings or the original input.
        assert!(f.controller.prepare_turn("thread-b").await.is_err());
        if let Err(error) = f.queue.kick_target("thread-b").await {
            assert!(
                matches!(error, QueueRunnerError::Backend(failure) if failure.kind == BackendFailureKind::AutoReserveHeld)
            );
        }
        assert_eq!(f.count("thread/settings/update"), 1);
        assert_eq!(f.count("turn/start"), 0);
        assert_eq!(
            reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap(),
            with_failure_fence
        );
        f.close().await;
    }
}

#[tokio::test]
async fn revision14_final_exhausted_reserve_holds_without_start_or_replay() {
    assert_final_rates_hold(json!({"used":100})).await;
}

#[tokio::test]
async fn revision14_final_spend_block_holds_without_start_or_replay() {
    assert_final_rates_hold(json!({"spend":true})).await;
}

#[tokio::test]
async fn revision14_final_rate_block_holds_without_start_or_replay() {
    assert_final_rates_hold(json!({"reached":"hardLimit"})).await;
}

#[tokio::test]
async fn revision14_final_unknown_capacity_holds_without_start_or_replay() {
    assert_final_rates_hold(json!({"used":null})).await;
}

#[tokio::test]
async fn revision14_final_missing_model_holds_without_start_or_replay() {
    assert_final_rates_hold(json!({"normal":"missing-model"})).await;
}

#[tokio::test]
async fn revision14_final_effort_mismatch_is_not_silently_reselected() {
    assert_final_rates_hold(json!({"normal":"model-b"})).await;
}
