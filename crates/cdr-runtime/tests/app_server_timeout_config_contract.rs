use std::collections::BTreeMap;
use std::time::Duration;

use cdr_runtime::config::{CliOptions, ConfigError, RuntimeConfig};

const RESUME_ENV: &str = "DISCORD_APP_SERVER_RESUME_TIMEOUT_SECONDS";
const HISTORY_READ_ENV: &str = "DISCORD_APP_SERVER_HISTORY_READ_TIMEOUT_SECONDS";

fn required_env() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("DISCORD_BOT_TOKEN".into(), "secret-token".into()),
        ("DISCORD_ALLOWED_CHANNEL_IDS".into(), "10".into()),
    ])
}

#[test]
fn app_server_timeouts_default_to_sixty_seconds_when_unset_or_blank() {
    for (name, raw) in [(RESUME_ENV, ""), (HISTORY_READ_ENV, "   ")] {
        let mut env = required_env();
        env.insert(name.into(), raw.into());

        let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

        assert_eq!(config.app_server_resume_timeout, Duration::from_mins(1));
        assert_eq!(
            config.app_server_history_read_timeout,
            Duration::from_mins(1)
        );
    }
}

#[test]
fn app_server_timeouts_accept_inclusive_boundaries_and_independent_values() {
    let mut env = required_env();
    env.extend([
        (RESUME_ENV.into(), "10".into()),
        (HISTORY_READ_ENV.into(), "300".into()),
    ]);

    let config = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap();

    assert_eq!(config.app_server_resume_timeout, Duration::from_secs(10));
    assert_eq!(
        config.app_server_history_read_timeout,
        Duration::from_mins(5)
    );
}

#[test]
fn malformed_app_server_timeout_is_rejected_with_its_variable_name() {
    for (name, raw) in [(RESUME_ENV, "not-a-number"), (HISTORY_READ_ENV, "10.5")] {
        let mut env = required_env();
        env.insert(name.into(), raw.into());

        let error = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap_err();

        assert_eq!(error, ConfigError::InvalidDurationSeconds(name));
    }
}

#[test]
fn out_of_range_app_server_timeout_is_rejected_instead_of_clamped() {
    for (name, raw) in [(RESUME_ENV, "9"), (HISTORY_READ_ENV, "301")] {
        let mut env = required_env();
        env.insert(name.into(), raw.into());

        let error = RuntimeConfig::from_map(&env, CliOptions::default()).unwrap_err();

        assert_eq!(
            error,
            ConfigError::DurationSecondsOutOfRange {
                name,
                minimum: 10,
                maximum: 300,
            }
        );
    }
}
