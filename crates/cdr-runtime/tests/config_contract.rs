use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use cdr_runtime::config::{CliOptions, ConfigError, RuntimeConfig, merge_env_text};

fn required_env() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("DISCORD_BOT_TOKEN".into(), " secret-token \n".into()),
        (
            "DISCORD_ALLOWED_CHANNEL_IDS".into(),
            "20, bad, 10,20".into(),
        ),
    ])
}

#[test]
fn defaults_and_id_sets_match_the_python_runtime() {
    let config = RuntimeConfig::from_map(&required_env(), CliOptions::default()).unwrap();

    assert_eq!(config.allowed_channel_ids, BTreeSet::from([10, 20]));
    assert_eq!(config.startup_channel_id, None);
    assert_eq!(config.bot_token.expose(), "secret-token");
    assert!(config.enable_message_content);
    assert!(config.stream_commentary);
    assert!(config.session_mirror);
    assert!(!config.qa_commands);
    assert!(!config.host_commands);
    assert!(!config.startup_notify);
    assert!(config.attachments_enabled);
    assert_eq!(config.attachment_max_bytes, 25 * 1024 * 1024);
    assert_eq!(config.attachment_text_inline_max_bytes, 32 * 1024);
    assert_eq!(config.history_poll_interval, Some(Duration::from_secs(15)));
}

#[test]
fn history_poll_interval_defaults_for_blank_or_invalid_values() {
    for raw in ["", "   ", "not-a-number", "NaN", "inf", "-inf"] {
        let mut env = required_env();
        env.insert("DISCORD_HISTORY_POLL_SECONDS".into(), raw.into());

        let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

        assert_eq!(config.history_poll_interval, Some(Duration::from_secs(15)));
    }
}

#[test]
fn history_poll_interval_clamps_and_zero_explicitly_disables_polling() {
    for (raw, expected) in [
        ("-1", None),
        ("-0.25", None),
        ("-0", None),
        ("0", None),
        ("1e-20", Some(Duration::from_nanos(1))),
        ("0.25", Some(Duration::from_millis(250))),
        ("999", Some(Duration::from_mins(5))),
    ] {
        let mut env = required_env();
        env.insert("DISCORD_HISTORY_POLL_SECONDS".into(), raw.into());

        let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

        assert_eq!(config.history_poll_interval, expected, "raw value: {raw}");
    }
}

#[test]
fn attachment_limits_match_python_defaults_and_bounds() {
    let mut env = required_env();
    env.extend([
        ("DISCORD_ENABLE_ATTACHMENTS".into(), "off".into()),
        ("DISCORD_ATTACHMENT_MAX_BYTES".into(), "999999999".into()),
        (
            "DISCORD_ATTACHMENT_TEXT_INLINE_MAX_BYTES".into(),
            "-1".into(),
        ),
    ]);
    let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();
    assert!(!config.attachments_enabled);
    assert_eq!(config.attachment_max_bytes, 100 * 1024 * 1024);
    assert_eq!(config.attachment_text_inline_max_bytes, 0);
}

#[test]
fn sole_allowed_channel_is_the_default_startup_channel() {
    let mut env = required_env();
    env.insert("DISCORD_ALLOWED_CHANNEL_IDS".into(), "44".into());

    let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

    assert_eq!(config.startup_channel_id, Some(44));
}

#[test]
fn explicit_startup_channel_and_optional_ids_are_loaded() {
    let mut env = required_env();
    env.extend([
        ("DISCORD_STARTUP_CHANNEL_ID".into(), "99".into()),
        ("DISCORD_GUILD_ID".into(), "77".into()),
        ("DISCORD_ALLOWED_USER_IDS".into(), "8,9".into()),
        ("DISCORD_PLAIN_ASK_MENTION_USER_IDS".into(), "6".into()),
    ]);

    let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

    assert_eq!(config.startup_channel_id, Some(99));
    assert_eq!(config.guild_id, Some(77));
    assert_eq!(config.allowed_user_ids, BTreeSet::from([8, 9]));
    assert_eq!(config.plain_ask_mention_user_ids, BTreeSet::from([6]));
}

#[test]
fn missing_token_and_missing_channel_gate_fail_without_secret_output() {
    let error = RuntimeConfig::from_map(&BTreeMap::new(), CliOptions::default()).unwrap_err();
    assert_eq!(error, ConfigError::MissingRequired("DISCORD_BOT_TOKEN"));

    let env = BTreeMap::from([("DISCORD_BOT_TOKEN".into(), "very-secret".into())]);
    let error = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap_err();
    assert_eq!(error, ConfigError::MissingAllowedChannels);
    assert!(!format!("{error:?}").contains("very-secret"));
}

#[test]
fn allow_all_channels_is_an_explicit_opt_in() {
    let env = BTreeMap::from([
        ("DISCORD_BOT_TOKEN".into(), "token".into()),
        ("DISCORD_ALLOW_ALL_CHANNELS".into(), "yes".into()),
    ]);

    let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

    assert!(config.allow_all_channels);
    assert!(config.allowed_channel_ids.is_empty());
}

#[test]
fn flags_use_python_truthy_and_falsey_rules_and_cli_can_disable_content() {
    let mut env = required_env();
    env.extend([
        ("DISCORD_ENABLE_QA_COMMANDS".into(), "yes".into()),
        ("DISCORD_ENABLE_HOST_COMMANDS".into(), "1".into()),
        ("DISCORD_STREAM_COMMENTARY".into(), "off".into()),
        ("DISCORD_STARTUP_NOTIFY".into(), "true".into()),
        ("DISCORD_SESSION_MIRROR".into(), "no".into()),
    ]);

    let config = RuntimeConfig::from_map(
        &env,
        CliOptions {
            no_message_content: true,
            check_config: false,
        },
    )
    .unwrap();

    assert!(config.qa_commands);
    assert!(config.host_commands);
    assert!(!config.stream_commentary);
    assert!(config.startup_notify);
    assert!(!config.session_mirror);
    assert!(!config.enable_message_content);
}

#[test]
fn env_message_content_flag_can_disable_prefix_and_plain_messages() {
    let mut env = required_env();
    env.insert("DISCORD_ENABLE_MESSAGE_CONTENT".into(), "0".into());

    let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

    assert!(!config.enable_message_content);
}

#[test]
fn env_file_values_fill_blanks_without_overwriting_process_values() {
    let mut env = BTreeMap::from([("EXISTING".into(), "process".into())]);
    merge_env_text(
        &mut env,
        "# ignored\n TOKEN = from-file\nDOUBLE = \"quoted\"\nSINGLE='single'\nNO_EQUALS\nEXISTING=file",
    );

    assert_eq!(env.get("TOKEN").map(String::as_str), Some("from-file"));
    assert_eq!(env.get("DOUBLE").map(String::as_str), Some("quoted"));
    assert_eq!(env.get("SINGLE").map(String::as_str), Some("single"));
    assert_eq!(env.get("EXISTING").map(String::as_str), Some("process"));
    assert!(!env.contains_key("NO_EQUALS"));
}

#[test]
fn invalid_scalar_ids_surface_the_actual_variable_name() {
    let mut env = required_env();
    env.insert("DISCORD_GUILD_ID".into(), "not-an-id".into());

    let error = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap_err();

    assert_eq!(error, ConfigError::InvalidInteger("DISCORD_GUILD_ID"));
}

#[test]
fn token_is_always_redacted_from_debug_output() {
    let config = RuntimeConfig::from_map(&required_env(), CliOptions::default()).unwrap();

    let debug = format!("{config:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("secret-token"));
}

#[test]
fn enabled_remote_mcp_is_validated_and_its_token_is_redacted() {
    let mut env = required_env();
    env.extend([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            "wss://gateway.example.test/bridge".into(),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "device-a".into()),
        (
            "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
            "remote-secret-token".into(),
        ),
    ]);

    let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

    assert_eq!(
        config
            .remote_mcp
            .as_ref()
            .map(|value| value.device_id.as_str()),
        Some("device-a")
    );
    let debug = format!("{config:?}");
    assert!(!debug.contains("remote-secret-token"));
}
