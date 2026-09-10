use cdr_discord::interaction::{InteractionRouteError, RoutedWork, route_command};
use cdr_runtime::command_plan::{CommandAction, plan_slash};
use twilight_model::application::{
    command::{CommandOptionType, CommandType},
    interaction::{
        InteractionType,
        application_command::{CommandData, CommandDataOption, CommandOptionValue},
    },
};
use twilight_model::id::Id;

fn slash(name: &str, options: Vec<CommandDataOption>) -> CommandAction {
    let data = CommandData {
        guild_id: None,
        id: Id::new(1),
        name: name.into(),
        kind: CommandType::ChatInput,
        options,
        resolved: None,
        target_id: None,
    };
    let route = route_command(&data, InteractionType::ApplicationCommand, true).unwrap();
    let Some(RoutedWork::Slash(invocation)) = route.work else {
        panic!("slash route expected");
    };
    plan_slash(&invocation).unwrap()
}

fn integer(name: &str, value: i64) -> CommandDataOption {
    CommandDataOption {
        name: name.into(),
        value: CommandOptionValue::Integer(value),
    }
}

fn string(name: &str, value: &str) -> CommandDataOption {
    CommandDataOption {
        name: name.into(),
        value: CommandOptionValue::String(value.into()),
    }
}

fn boolean(name: &str, value: bool) -> CommandDataOption {
    CommandDataOption {
        name: name.into(),
        value: CommandOptionValue::Boolean(value),
    }
}

#[test]
fn list_usage_and_context_limits_match_python_clamping() {
    assert_eq!(slash("list", vec![]), CommandAction::List { limit: 10 });
    assert_eq!(
        slash("list", vec![integer("limit", 99)]),
        CommandAction::List { limit: 30 }
    );
    assert_eq!(
        slash("archived_list", vec![integer("limit", -1)]),
        CommandAction::ArchivedList { limit: 1 }
    );
    assert_eq!(
        slash("usage", vec![integer("days", 99)]),
        CommandAction::Usage { days: 30 }
    );
    assert_eq!(
        slash(
            "context",
            vec![
                boolean("all_threads", true),
                boolean("refresh", true),
                integer("limit", 0)
            ],
        ),
        CommandAction::Context {
            all_threads: true,
            refresh: true,
            limit: 1
        }
    );
}

#[test]
fn target_and_settings_options_are_trimmed_but_not_guessed() {
    assert_eq!(
        slash("use", vec![string("ref", "  abc ")]),
        CommandAction::Use {
            reference: "abc".into()
        }
    );
    assert_eq!(
        slash("status", vec![]),
        CommandAction::Status { reference: None }
    );
    assert_eq!(
        slash(
            "settings",
            vec![
                string("ref", "thread-a"),
                string("model", "gpt-5.6"),
                string("effort", "high"),
                string("speed", "fast"),
            ],
        ),
        CommandAction::Settings {
            reference: Some("thread-a".into()),
            model: Some("gpt-5.6".into()),
            effort: Some("high".into()),
            speed: Some("fast".into()),
        }
    );
}

#[test]
fn prompt_and_runtime_commands_cover_the_complete_registered_inventory() {
    assert_eq!(slash("help", vec![]), CommandAction::Help);
    assert_eq!(slash("where", vec![]), CommandAction::Where);
    assert_eq!(
        slash("new", vec![string("prompt", "p")]),
        CommandAction::New { prompt: "p".into() }
    );
    assert_eq!(
        slash("ask", vec![string("prompt", "p")]),
        CommandAction::Ask { prompt: "p".into() }
    );
    assert_eq!(
        slash("interview", vec![string("prompt", "p")]),
        CommandAction::Interview { prompt: "p".into() }
    );
    assert_eq!(slash("doctor", vec![]), CommandAction::Doctor);
    assert_eq!(slash("approval", vec![]), CommandAction::Approval);
    assert_eq!(slash("runners", vec![]), CommandAction::Runners);
    assert_eq!(
        slash("retract", vec![]),
        CommandAction::Retract { reference: None }
    );
    assert_eq!(slash("mirror_check", vec![]), CommandAction::MirrorCheck);
    assert_eq!(
        slash("bridge_sync", vec![]),
        CommandAction::BridgeSync { limit: None }
    );
    assert_eq!(slash("qa_buttons", vec![]), CommandAction::QaButtons);
}

#[test]
fn ipc_named_legacy_command_is_not_supported_by_the_app_server_only_runtime() {
    let data = CommandData {
        guild_id: None,
        id: Id::new(1),
        name: "ask_ipc".into(),
        kind: CommandType::ChatInput,
        options: vec![string("prompt", "p")],
        resolved: None,
        target_id: None,
    };
    assert_eq!(
        route_command(&data, InteractionType::ApplicationCommand, true),
        Err(InteractionRouteError::UnknownCommand("ask_ipc".into()))
    );
}

#[test]
fn command_option_type_fixture_still_uses_integer_for_limits() {
    assert_eq!(
        CommandOptionValue::Integer(2).kind(),
        CommandOptionType::Integer
    );
}
