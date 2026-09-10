use std::collections::BTreeSet;

use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::message_plan::{IncomingMessage, MessagePlan, MessagePlanError, plan_message};

fn ids(values: &[u64]) -> BTreeSet<u64> {
    values.iter().copied().collect()
}

fn input(content: &str) -> IncomingMessage<'_> {
    IncomingMessage {
        content,
        message_content_enabled: true,
        channel_allowed: true,
        user_allowed: true,
        author_is_bot: false,
        author_is_self: false,
        author_mentions_bridge: false,
        has_attachments: false,
        mirrored_target: false,
        mentioned_user_ids: BTreeSet::new(),
        required_plain_ask_user_ids: BTreeSet::new(),
    }
}

#[test]
fn access_and_bot_messages_are_rejected_before_parsing() {
    let mut denied = input("!help");
    denied.user_allowed = false;
    assert_eq!(
        plan_message(&denied).unwrap(),
        MessagePlan::Ignore("user_not_allowed")
    );

    let mut self_message = input("hello");
    self_message.author_is_bot = true;
    self_message.author_is_self = true;
    assert_eq!(
        plan_message(&self_message).unwrap(),
        MessagePlan::Ignore("self_authored")
    );

    let mut bridge = input("!help");
    bridge.author_is_bot = true;
    bridge.author_mentions_bridge = true;
    assert_eq!(
        plan_message(&bridge).unwrap(),
        MessagePlan::Execute(CommandAction::Help)
    );
}

#[test]
fn prefix_commands_use_the_existing_exact_grammar() {
    assert_eq!(
        plan_message(&input("  !list 99 ")).unwrap(),
        MessagePlan::Execute(CommandAction::List { limit: 30 })
    );
    assert!(matches!(
        plan_message(&input("!not-a-command")),
        Err(MessagePlanError::Prefix(_))
    ));
    assert_eq!(
        plan_message(&input("!restart_codex")).unwrap(),
        MessagePlan::Execute(CommandAction::RestartCodex)
    );
}

#[test]
fn direct_plain_ask_requires_and_strips_configured_mention() {
    let mut missing = input("please continue");
    missing.required_plain_ask_user_ids = ids(&[42]);
    assert_eq!(
        plan_message(&missing).unwrap(),
        MessagePlan::Ignore("required_mention_missing")
    );

    let mut mentioned = input("<@42> please continue");
    mentioned.required_plain_ask_user_ids = ids(&[42]);
    mentioned.mentioned_user_ids = ids(&[42]);
    assert_eq!(
        plan_message(&mentioned).unwrap(),
        MessagePlan::Execute(CommandAction::Ask {
            prompt: "please continue".into()
        })
    );
}

#[test]
fn mirrored_plain_ask_bypasses_direct_mention_and_attachment_only_is_explicit() {
    let mut mirrored = input("continue");
    mirrored.required_plain_ask_user_ids = ids(&[42]);
    mirrored.mirrored_target = true;
    assert!(matches!(
        plan_message(&mirrored).unwrap(),
        MessagePlan::Execute(CommandAction::Ask { .. })
    ));

    let mut attachment = input("");
    attachment.has_attachments = true;
    assert_eq!(
        plan_message(&attachment).unwrap(),
        MessagePlan::Execute(CommandAction::Ask {
            prompt: "Please inspect the attached Discord file(s).".into()
        })
    );
}
