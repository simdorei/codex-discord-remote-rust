use cdr_runtime::prefix_plan::{PrefixAction, plan_prefix};

#[test]
fn blank_slash_setting_is_rejected_instead_of_silently_becoming_a_query() {
    for field in ["model", "effort", "speed"] {
        let data = serde_json::from_value(serde_json::json!({"id":"1","name":"settings","type":1,
            "options":[{"name":field,"type":3,"value":"   "}]}))
        .unwrap();
        let route = cdr_discord::interaction::route_command(
            &data,
            twilight_model::application::interaction::InteractionType::ApplicationCommand,
            true,
        )
        .unwrap();
        let Some(cdr_discord::interaction::RoutedWork::Slash(invocation)) = route.work else {
            panic!("slash");
        };
        assert!(
            cdr_runtime::command_plan::plan_slash(&invocation).is_err(),
            "blank {field} must not be omitted"
        );
    }
}

#[test]
fn no_values_query_the_exact_requested_thread_settings() {
    for (command, reference) in [
        ("settings", None),
        ("settings thread-b", Some("thread-b")),
        ("setting thread-b", Some("thread-b")),
    ] {
        assert_eq!(
            plan_prefix(command).unwrap(),
            PrefixAction::Settings {
                reference: reference.map(str::to_owned),
                model: None,
                effort: None,
                speed: None,
            }
        );
    }
}

#[test]
fn valueless_option_preserves_its_explicit_reference() {
    assert_eq!(
        plan_prefix("settings thread-b --model").unwrap(),
        PrefixAction::SettingsOptions {
            reference: Some("thread-b".into()),
            field: Some("model".into())
        }
    );
}

#[test]
fn incomplete_mixed_duplicate_or_unknown_options_never_become_a_partial_change() {
    for command in [
        "settings --model --speed fast",
        "settings --speed fast --model",
        "settings --model --mystery bad",
        "settings --model first --model second",
        "settings --reasoning high --effort low",
        "settings --model \"\"",
        "settings --speed fast --speed standard",
        "settings \"\"",
    ] {
        assert!(
            plan_prefix(command).is_err(),
            "must reject the complete malformed command: {command}"
        );
    }
}
