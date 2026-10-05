use std::collections::BTreeSet;

use cdr_runtime::{
    command_plan::CommandAction,
    message_plan,
    prefix_plan::{PrefixAction, plan_prefix},
};

#[test]
fn repair_is_separate_from_recover_and_requires_an_authorized_human() {
    for name in ["repair", "도구복구"] {
        assert_eq!(
            plan_prefix(name).unwrap(),
            PrefixAction::Repair { reference: None }
        );
        assert_eq!(
            plan_prefix(&format!("{name} thread-a")).unwrap(),
            PrefixAction::Repair {
                reference: Some("thread-a".into())
            }
        );
        assert!(plan_prefix(&format!("{name} a b")).is_err());
        assert!(cdr_discord::gateway::ingress::is_emergency_message(
            &format!("!{name} thread-a")
        ));
    }
    let mut input = message_plan::IncomingMessage {
        content: "!repair thread-a",
        message_content_enabled: true,
        channel_allowed: true,
        user_allowed: true,
        author_is_bot: false,
        author_is_self: false,
        author_mentions_bridge: false,
        has_attachments: false,
        mirrored_target: true,
        mentioned_user_ids: BTreeSet::default(),
        required_plain_ask_user_ids: BTreeSet::default(),
    };
    assert_eq!(
        message_plan::plan_message(&input).unwrap(),
        message_plan::MessagePlan::Execute(CommandAction::Repair {
            reference: Some("thread-a".into())
        })
    );
    input.author_is_bot = true;
    assert!(matches!(
        message_plan::plan_message(&input).unwrap(),
        message_plan::MessagePlan::Ignore(_)
    ));
    input.author_is_bot = false;
    input.user_allowed = false;
    assert!(matches!(
        message_plan::plan_message(&input).unwrap(),
        message_plan::MessagePlan::Ignore(_)
    ));
    let help = include_str!("../src/action_executor/help.txt");
    assert!(help.contains("!repair [ref]"));
    assert!(help.contains("앱 재시작·요청 취소 없음"));
    assert!(help.contains("!recover [ref]"));
    for invalid in ["repair", "!repairing", "!repair a b"] {
        assert!(!cdr_discord::gateway::ingress::is_emergency_message(
            invalid
        ));
    }
}

#[test]
#[cfg(windows)]
fn full_recovery_restarts_pinned_hosts_and_restores_desktop_after_bot_failure() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts/Test-CdrToolsRecovery.ps1"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("full_recovery_tests_passed"));
}

#[test]
fn recovery_is_a_direct_control_command_and_is_documented() {
    for text in ["recover", "복구"] {
        assert_eq!(
            plan_prefix(text).unwrap(),
            PrefixAction::Recover { reference: None }
        );
        assert_eq!(
            plan_prefix(&format!("{text} thread-a")).unwrap(),
            PrefixAction::Recover {
                reference: Some("thread-a".into())
            }
        );
    }
    let help = include_str!("../src/action_executor/help.txt");
    assert!(help.contains("!recover [ref]"));
    assert!(help.contains("다른 앱 채팅도 중단"));
    let action = CommandAction::Recover {
        reference: Some("thread-a".into()),
    };
    let value = serde_json::to_value(&action).unwrap();
    assert_eq!(
        serde_json::from_value::<CommandAction>(value).unwrap(),
        action
    );
    let mut input = message_plan::IncomingMessage {
        content: "!recover thread-a",
        message_content_enabled: true,
        channel_allowed: true,
        user_allowed: true,
        author_is_bot: false,
        author_is_self: false,
        author_mentions_bridge: false,
        has_attachments: false,
        mirrored_target: true,
        mentioned_user_ids: BTreeSet::default(),
        required_plain_ask_user_ids: BTreeSet::default(),
    };
    assert_eq!(
        message_plan::plan_message(&input).unwrap(),
        message_plan::MessagePlan::Execute(action)
    );
    input.author_is_bot = true;
    input.author_mentions_bridge = true;
    assert!(matches!(
        message_plan::plan_message(&input).unwrap(),
        message_plan::MessagePlan::Ignore(_)
    ));
    input.author_is_bot = false;
    input.user_allowed = false;
    assert!(matches!(
        message_plan::plan_message(&input).unwrap(),
        message_plan::MessagePlan::Ignore(_)
    ));
    for text in [
        "!recover",
        "!RECOVER thread-a",
        "!복구 thread-a",
        "!force_restart",
    ] {
        assert!(cdr_discord::gateway::ingress::is_emergency_message(text));
    }
    for text in ["recover", "!recover a b", "!recovering"] {
        assert!(!cdr_discord::gateway::ingress::is_emergency_message(text));
    }
    assert!(plan_prefix("recover a b").is_err());
}

#[test]
#[cfg(windows)]
fn exact_owner_desktop_recovery_preserves_unrelated_processes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts/Test-CdrDesktopRecovery.ps1"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("desktop_recovery_tests_passed"));
}
