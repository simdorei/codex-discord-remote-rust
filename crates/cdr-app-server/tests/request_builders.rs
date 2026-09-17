use std::time::Duration;

use cdr_app_server::requests::{
    ServiceTierUpdate, ThreadSettingsUpdate, archive_thread, clean_background_terminals,
    fork_thread_persistent, get_goal, interrupt_turn, list_models, rate_limits, read_thread,
    read_thread_with_timeout, resume_thread, resume_thread_with_timeout, start_thread, start_turn,
    start_turn_with_input, steer_turn, unsubscribe_thread, update_thread_settings, usage,
};
use serde_json::json;

#[test]
fn read_resume_and_start_thread_requests_match_the_python_wire_contract() {
    let read = read_thread("thread-a", true);
    assert_eq!(read.method, "thread/read");
    assert_eq!(
        read.params,
        json!({"threadId":"thread-a", "includeTurns":true})
    );
    assert_eq!(read.timeout, Duration::from_secs(8));

    let resume = resume_thread("thread-a");
    assert_eq!(resume.method, "thread/resume");
    assert_eq!(resume.params, json!({"threadId":"thread-a"}));
    assert_eq!(resume.timeout, Duration::from_secs(10));

    assert_eq!(start_thread(None).params, json!({}));
    assert_eq!(
        start_thread(Some("C:/repo")).params,
        json!({"cwd":"C:/repo"})
    );
}

#[test]
fn thread_read_and_resume_accept_the_callers_exact_timeout() {
    let read_timeout = Duration::from_millis(12_345);
    let read = read_thread_with_timeout("thread-a", false, read_timeout);
    assert_eq!(read.method, "thread/read");
    assert_eq!(
        read.params,
        json!({"threadId":"thread-a", "includeTurns":false})
    );
    assert_eq!(read.timeout, read_timeout);

    let resume_timeout = Duration::from_millis(67_890);
    let resume = resume_thread_with_timeout("thread-a", resume_timeout);
    assert_eq!(resume.method, "thread/resume");
    assert_eq!(resume.params, json!({"threadId":"thread-a"}));
    assert_eq!(resume.timeout, resume_timeout);
}

#[test]
fn persistent_thread_fork_is_explicit_and_uses_the_callers_exact_timeout() {
    let timeout = Duration::from_millis(98_765);
    let fork = fork_thread_persistent("thread-a", timeout);

    assert_eq!(fork.method, "thread/fork");
    assert_eq!(
        fork.params,
        json!({"threadId":"thread-a", "ephemeral":false})
    );
    assert_eq!(fork.timeout, timeout);
}

#[test]
fn turn_requests_use_current_text_input_shape_and_generation_safe_identity() {
    let start = start_turn("thread-a", "hello");
    assert_eq!(start.method, "turn/start");
    assert_eq!(start.timeout, Duration::from_secs(12));
    assert_eq!(
        start.params,
        json!({"threadId":"thread-a", "input":[{"type":"text", "text":"hello", "text_elements":[]}]})
    );
    let rich = start_turn_with_input(
        "thread-a",
        &[
            json!({"type":"text", "text":"hello", "text_elements":[]}),
            json!({"type":"skill", "name":"ask-chatgpt-pro", "path":"C:/skill/SKILL.md"}),
        ],
    );
    assert_eq!(
        rich.params,
        json!({"threadId":"thread-a", "input":[
            {"type":"text", "text":"hello", "text_elements":[]},
            {"type":"skill", "name":"ask-chatgpt-pro", "path":"C:/skill/SKILL.md"}
        ]})
    );

    let steer = steer_turn("thread-a", "continue", "turn-a");
    assert_eq!(steer.method, "turn/steer");
    assert_eq!(
        steer.params,
        json!({"threadId":"thread-a", "expectedTurnId":"turn-a", "input":[{"type":"text", "text":"continue", "text_elements":[]}]})
    );

    assert_eq!(
        interrupt_turn("thread-a", "turn-a").params,
        json!({"threadId":"thread-a", "turnId":"turn-a"})
    );
}

#[test]
fn settings_preserve_unchanged_set_and_explicit_standard_tier() {
    let update = update_thread_settings(
        "thread-a",
        &ThreadSettingsUpdate {
            model: Some("gpt-5.6".into()),
            effort: Some("high".into()),
            effort_clear: false,
            service_tier: ServiceTierUpdate::Clear,
        },
    );

    assert_eq!(update.method, "thread/settings/update");
    assert_eq!(
        update.params,
        json!({"threadId":"thread-a", "model":"gpt-5.6", "effort":"high", "serviceTier":null})
    );

    let set = update_thread_settings(
        "thread-a",
        &ThreadSettingsUpdate {
            service_tier: ServiceTierUpdate::Set("priority".into()),
            ..ThreadSettingsUpdate::default()
        },
    );
    assert_eq!(
        set.params,
        json!({"threadId":"thread-a", "serviceTier":"priority"})
    );
}

#[test]
fn every_observed_non_turn_operation_has_an_exact_builder() {
    let cases = [
        (list_models(), "model/list", json!({}), 8),
        (get_goal("t"), "thread/goal/get", json!({"threadId":"t"}), 8),
        (
            archive_thread("t"),
            "thread/archive",
            json!({"threadId":"t"}),
            10,
        ),
        (
            clean_background_terminals("t"),
            "thread/backgroundTerminals/clean",
            json!({"threadId":"t"}),
            10,
        ),
        (
            unsubscribe_thread("t"),
            "thread/unsubscribe",
            json!({"threadId":"t"}),
            8,
        ),
        (rate_limits(), "account/rateLimits/read", json!({}), 15),
        (usage(), "account/usage/read", json!({}), 15),
    ];
    for (request, method, params, seconds) in cases {
        assert_eq!(request.method, method);
        assert_eq!(request.params, params);
        assert_eq!(request.timeout, Duration::from_secs(seconds));
    }
}
