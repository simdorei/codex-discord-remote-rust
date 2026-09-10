use cdr_discord::interaction::{InteractionRouteError, RoutedWork, route_command, route_component};
use twilight_model::{
    application::interaction::{
        InteractionType, application_command::CommandData,
        message_component::MessageComponentInteractionData,
    },
    http::interaction::InteractionResponseType,
};

fn command(value: serde_json::Value) -> CommandData {
    serde_json::from_value(value).expect("command data")
}

#[test]
fn slash_routes_with_deferred_ack_and_typed_values() {
    let data = command(serde_json::json!({
        "id": "1",
        "name": "context",
        "type": 1,
        "options": [
            {"name": "all_threads", "type": 5, "value": true},
            {"name": "limit", "type": 4, "value": 15}
        ]
    }));
    let route = route_command(&data, InteractionType::ApplicationCommand, false).expect("route");
    assert_eq!(
        route.initial_response.kind,
        InteractionResponseType::DeferredChannelMessageWithSource
    );
    let RoutedWork::Slash(invocation) = route.work.expect("slash work") else {
        panic!("expected slash work");
    };
    assert_eq!(invocation.name, "context");
    assert_eq!(invocation.boolean("all_threads"), Some(true));
    assert_eq!(invocation.integer("limit"), Some(15));
    assert_eq!(invocation.string("limit"), None);
}

#[test]
fn command_schema_rejects_unknown_missing_duplicate_and_wrong_typed_options() {
    let cases = [
        serde_json::json!({"id":"1","name":"nope","type":1}),
        serde_json::json!({"id":"1","name":"use","type":1}),
        serde_json::json!({"id":"1","name":"use","type":1,"options":[
            {"name":"ref","type":4,"value":1}
        ]}),
        serde_json::json!({"id":"1","name":"status","type":1,"options":[
            {"name":"ref","type":3,"value":"a"},
            {"name":"ref","type":3,"value":"b"}
        ]}),
        serde_json::json!({"id":"1","name":"status","type":1,"options":[
            {"name":"extra","type":3,"value":"a"}
        ]}),
    ];
    for value in cases {
        let data = command(value);
        assert!(route_command(&data, InteractionType::ApplicationCommand, false).is_err());
    }
}

#[test]
fn qa_command_is_registered_and_routable_only_when_enabled() {
    let data = command(serde_json::json!({"id":"1","name":"qa_buttons","type":1}));
    assert_eq!(
        route_command(&data, InteractionType::ApplicationCommand, false),
        Err(InteractionRouteError::UnknownCommand("qa_buttons".into()))
    );
    assert!(route_command(&data, InteractionType::ApplicationCommand, true).is_ok());
}

#[test]
fn settings_autocomplete_is_immediate_and_other_focused_options_fail_closed() {
    let data = command(serde_json::json!({
        "id":"1","name":"settings","type":1,
        "options":[{"name":"model","type":3,"value":"gpt","focused":true}]
    }));
    let route = route_command(
        &data,
        InteractionType::ApplicationCommandAutocomplete,
        false,
    )
    .expect("autocomplete route");
    assert_eq!(
        route.initial_response.kind,
        InteractionResponseType::ApplicationCommandAutocompleteResult
    );
    let RoutedWork::Autocomplete(focused) = route.work.expect("autocomplete work") else {
        panic!("expected autocomplete work");
    };
    assert_eq!(focused.command_name, "settings");
    assert_eq!(focused.option_name, "model");
    assert_eq!(focused.current, "gpt");

    let invalid = command(serde_json::json!({
        "id":"1","name":"status","type":1,
        "options":[{"name":"ref","type":3,"value":"x","focused":true}]
    }));
    assert!(
        route_command(
            &invalid,
            InteractionType::ApplicationCommandAutocomplete,
            false
        )
        .is_err()
    );
}

#[test]
fn persistent_component_routes_with_deferred_update_ack() {
    let data = MessageComponentInteractionData {
        custom_id: "codex_approval:thread-1:1".into(),
        component_type: twilight_model::channel::message::component::ComponentType::Button,
        resolved: None,
        values: Vec::new(),
    };
    let route = route_component(&data).expect("component route");
    assert_eq!(
        route.initial_response.kind,
        InteractionResponseType::DeferredUpdateMessage
    );
    assert!(matches!(route.work, Some(RoutedWork::Component(_))));
}
