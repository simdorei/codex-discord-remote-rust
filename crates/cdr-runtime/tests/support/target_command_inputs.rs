use cdr_runtime::{
    command_plan::CommandAction,
    message_plan::{IncomingMessage, MessagePlan, plan_message},
};

pub fn prefix(text: &str) -> CommandAction {
    let input = IncomingMessage {
        content: text,
        message_content_enabled: true,
        channel_allowed: true,
        user_allowed: true,
        author_is_bot: false,
        author_is_self: false,
        author_mentions_bridge: false,
        has_attachments: false,
        mirrored_target: true,
        mentioned_user_ids: std::collections::BTreeSet::new(),
        required_plain_ask_user_ids: std::collections::BTreeSet::new(),
    };
    let MessagePlan::Execute(action) = plan_message(&input).unwrap() else {
        panic!("expected routed command")
    };
    action
}

pub fn slash_status(reference: &str) -> CommandAction {
    slash_reference("status", Some(reference)).unwrap()
}

pub fn slash_reference(
    name: &str,
    reference: Option<&str>,
) -> Result<CommandAction, cdr_runtime::command_plan::CommandPlanError> {
    use cdr_discord::interaction::{RoutedWork, route_command};
    use twilight_model::application::{
        command::CommandType,
        interaction::{
            InteractionType,
            application_command::{CommandData, CommandDataOption, CommandOptionValue},
        },
    };
    let data = CommandData {
        guild_id: None,
        id: twilight_model::id::Id::new(1),
        name: name.into(),
        kind: CommandType::ChatInput,
        options: reference
            .map(|reference| CommandDataOption {
                name: "ref".into(),
                value: CommandOptionValue::String(reference.into()),
            })
            .into_iter()
            .collect(),
        resolved: None,
        target_id: None,
    };
    let route = route_command(&data, InteractionType::ApplicationCommand, false).unwrap();
    let Some(RoutedWork::Slash(invocation)) = route.work else {
        panic!("expected slash command")
    };
    cdr_runtime::command_plan::plan_slash(&invocation)
}
