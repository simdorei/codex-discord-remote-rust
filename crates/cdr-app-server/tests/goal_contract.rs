use cdr_app_server::goal::{
    ThreadGoalStatus, is_terminal_goal_status, parse_thread_goal_status, parse_thread_goal_update,
};
use serde_json::json;

#[test]
fn goal_get_distinguishes_absent_active_and_complete() {
    assert_eq!(
        parse_thread_goal_status(&json!({"goal":null}), "t").unwrap(),
        None
    );
    assert_eq!(
        parse_thread_goal_status(&json!({"goal":{"threadId":"t", "status":"active"}}), "t")
            .unwrap(),
        Some(ThreadGoalStatus::Active)
    );
    assert!(is_terminal_goal_status(ThreadGoalStatus::Complete));
    assert!(is_terminal_goal_status(ThreadGoalStatus::Blocked));
    assert!(!is_terminal_goal_status(ThreadGoalStatus::Paused));
}

#[test]
fn goal_identity_unknown_status_and_update_shapes_fail_closed() {
    assert!(
        parse_thread_goal_status(
            &json!({"goal":{"threadId":"other", "status":"active"}}),
            "t"
        )
        .unwrap_err()
        .to_string()
        .contains("different thread")
    );
    assert!(
        parse_thread_goal_status(&json!({"goal":{"threadId":"t", "status":"future"}}), "t")
            .unwrap_err()
            .to_string()
            .contains("unknown goal status")
    );
    let update = parse_thread_goal_update(&json!({
        "threadId":"t", "turnId":" turn-a ", "goal":{"threadId":"t", "status":"usageLimited"}
    }))
    .unwrap();
    assert_eq!(update.thread_id, "t");
    assert_eq!(update.turn_id.as_deref(), Some("turn-a"));
    assert_eq!(update.status, ThreadGoalStatus::UsageLimited);
}
