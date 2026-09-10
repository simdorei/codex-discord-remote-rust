use cdr_runtime::{
    command_plan::CommandAction,
    message_plan::{IncomingMessage, MessagePlan, plan_message},
};
use std::collections::BTreeSet;

#[test]
fn actual_prefix_keeps_the_pro_command_for_authoritative_preprocessing() {
    for request in ["!pro 핑퐁", "!pro review 코드\n검수", "!PRO review 확인"] {
        let input = IncomingMessage {
            content: request,
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
        };
        let MessagePlan::Execute(CommandAction::Ask { prompt }) = plan_message(&input).unwrap()
        else {
            panic!("ask path")
        };
        assert_eq!(
            cdr_pro::prompt::rewrite_pro_prompt(&prompt),
            cdr_pro::prompt::rewrite_pro_prompt(request)
        );
        assert!(cdr_pro::prompt::is_pro_command(&prompt));
    }
}
