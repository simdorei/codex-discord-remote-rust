use cdr_runtime::message_plan::{IncomingMessage, MessagePlan, plan_message};
use std::collections::BTreeSet;

fn plan(text: &str) -> MessagePlan {
    plan_message(&input(text)).unwrap()
}

fn input(text: &str) -> IncomingMessage<'_> {
    IncomingMessage {
        content: text,
        message_content_enabled: true,
        channel_allowed: true,
        user_allowed: true,
        author_is_bot: false,
        author_is_self: false,
        author_mentions_bridge: false,
        has_attachments: false,
        mirrored_target: true,
        mentioned_user_ids: BTreeSet::new(),
        required_plain_ask_user_ids: BTreeSet::new(),
    }
}

#[test]
fn advertised_aliases_reach_the_same_command_with_the_same_arguments() {
    for (alias, canonical) in [
        ("!archive_list 7", "!archived_list 7"),
        ("!setting --model", "!settings --model"),
        ("!map", "!where"),
        ("!ctx recent 3", "!context refresh 3"),
        ("!quota 14", "!usage 14"),
        ("!limit 14", "!usage 14"),
        ("!queues", "!runners"),
        ("!queues message:123", "!runners message:123"),
        ("!system", "!resources"),
        ("!unqueue thread-b", "!retract thread-b"),
        ("!resync 8", "!bridge_sync 8"),
        ("!sync 8", "!bridge_sync 8"),
        ("!bridge sync 8", "!bridge_sync 8"),
        ("!approve", "!approval"),
        ("!deep_interview 요구 사항", "!interview 요구 사항"),
        ("!deep-interview 요구 사항", "!interview 요구 사항"),
    ] {
        let expected = plan(canonical);
        assert!(matches!(expected, MessagePlan::Execute(_)));
        assert_eq!(plan(alias), expected, "{alias} != {canonical}");
    }
}

#[test]
fn whitespace_command_boundaries_preserve_the_prompt_and_nested_arguments() {
    for (input, canonical) in [
        ("!new\n첫 요청\n둘째 줄", "!new 첫 요청\n둘째 줄"),
        ("!steer\t추가 지시", "!steer 추가 지시"),
        ("!bridge\tsync\t8", "!bridge sync 8"),
        ("!mirror\tcheck\t8", "!mirror check 8"),
    ] {
        assert_eq!(plan(input), plan(canonical));
    }
}

#[test]
fn unimplemented_detail_does_not_report_success_as_an_unrelated_mirror_check() {
    for command in ["!detail", "!detail send", "!detail all"] {
        assert!(
            matches!(
                plan_message(&input(command)),
                Err(cdr_runtime::message_plan::MessagePlanError::UnsupportedPrefix("detail"))
            ),
            "{command} was silently replaced"
        );
    }
}
