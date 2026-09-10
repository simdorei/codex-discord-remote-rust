use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::time::Duration;

use cdr_remote_agent::config::{ConfigError as RemoteConfigError, RemoteMcpConfig};
use thiserror::Error;

const FALSE_VALUES: [&str; 4] = ["0", "false", "no", "off"];
const APP_SERVER_TIMEOUT_DEFAULT_SECONDS: u64 = 60;
const APP_SERVER_TIMEOUT_MINIMUM_SECONDS: u64 = 10;
const APP_SERVER_TIMEOUT_MAXIMUM_SECONDS: u64 = 300;

#[derive(Clone, Default, Eq, PartialEq)]
pub struct SecretString(String);

impl SecretString {
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CliOptions {
    pub no_message_content: bool,
    pub check_config: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "these are independent deployment switches mirrored from the Python runtime"
)]
pub struct RuntimeConfig {
    pub bot_token: SecretString,
    pub remote_mcp: Option<RemoteMcpConfig>,
    pub allowed_channel_ids: BTreeSet<u64>,
    pub allowed_user_ids: BTreeSet<u64>,
    pub plain_ask_mention_user_ids: BTreeSet<u64>,
    pub startup_channel_id: Option<u64>,
    pub guild_id: Option<u64>,
    pub allow_all_channels: bool,
    pub enable_message_content: bool,
    pub qa_commands: bool,
    pub host_commands: bool,
    pub stream_commentary: bool,
    pub startup_notify: bool,
    pub session_mirror: bool,
    pub attachments_enabled: bool,
    pub attachment_max_bytes: u64,
    pub attachment_text_inline_max_bytes: u64,
    pub history_poll_interval: Option<Duration>,
    pub app_server_resume_timeout: Duration,
    pub app_server_history_read_timeout: Duration,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    MissingRequired(&'static str),
    #[error("set DISCORD_ALLOWED_CHANNEL_IDS or DISCORD_ALLOW_ALL_CHANNELS=1")]
    MissingAllowedChannels,
    #[error("invalid integer in environment variable: {0}")]
    InvalidInteger(&'static str),
    #[error("invalid duration seconds in environment variable: {0}")]
    InvalidDurationSeconds(&'static str),
    #[error(
        "duration seconds in environment variable {name} must be between {minimum} and {maximum} inclusive"
    )]
    DurationSecondsOutOfRange {
        name: &'static str,
        minimum: u64,
        maximum: u64,
    },
    #[error(transparent)]
    RemoteMcp(#[from] RemoteConfigError),
}

impl RuntimeConfig {
    pub fn from_map(env: &BTreeMap<String, String>, cli: CliOptions) -> Result<Self, ConfigError> {
        let bot_token = required(env, "DISCORD_BOT_TOKEN")?;
        let allowed_channel_ids = int_set(env, "DISCORD_ALLOWED_CHANNEL_IDS");
        let allow_all_channels = flag(env, "DISCORD_ALLOW_ALL_CHANNELS", false);
        if allowed_channel_ids.is_empty() && !allow_all_channels {
            return Err(ConfigError::MissingAllowedChannels);
        }
        let startup_channel_id = optional_integer(env, "DISCORD_STARTUP_CHANNEL_ID")?
            .or_else(|| sole_value(&allowed_channel_ids));
        let env_message_content = flag(env, "DISCORD_ENABLE_MESSAGE_CONTENT", true);
        let remote_values = env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<HashMap<_, _>>();
        Ok(Self {
            bot_token: SecretString(bot_token.to_owned()),
            remote_mcp: cdr_remote_agent::config::load_remote_mcp_config(&remote_values)?,
            allowed_channel_ids,
            allowed_user_ids: int_set(env, "DISCORD_ALLOWED_USER_IDS"),
            plain_ask_mention_user_ids: int_set(env, "DISCORD_PLAIN_ASK_MENTION_USER_IDS"),
            startup_channel_id,
            guild_id: optional_integer(env, "DISCORD_GUILD_ID")?,
            allow_all_channels,
            enable_message_content: env_message_content && !cli.no_message_content,
            qa_commands: flag(env, "DISCORD_ENABLE_QA_COMMANDS", false),
            host_commands: flag(env, "DISCORD_ENABLE_HOST_COMMANDS", false),
            stream_commentary: flag(env, "DISCORD_STREAM_COMMENTARY", true),
            startup_notify: flag(env, "DISCORD_STARTUP_NOTIFY", false),
            session_mirror: flag(env, "DISCORD_SESSION_MIRROR", true),
            attachments_enabled: flag(env, "DISCORD_ENABLE_ATTACHMENTS", true),
            attachment_max_bytes: bounded_integer(
                env,
                "DISCORD_ATTACHMENT_MAX_BYTES",
                25 * 1024 * 1024,
                1,
                100 * 1024 * 1024,
            ),
            attachment_text_inline_max_bytes: bounded_integer(
                env,
                "DISCORD_ATTACHMENT_TEXT_INLINE_MAX_BYTES",
                32 * 1024,
                0,
                1024 * 1024,
            ),
            history_poll_interval: optional_bounded_duration(
                env,
                "DISCORD_HISTORY_POLL_SECONDS",
                15.0,
                300.0,
            ),
            app_server_resume_timeout: strict_bounded_duration_seconds(
                env,
                "DISCORD_APP_SERVER_RESUME_TIMEOUT_SECONDS",
                APP_SERVER_TIMEOUT_DEFAULT_SECONDS,
                APP_SERVER_TIMEOUT_MINIMUM_SECONDS,
                APP_SERVER_TIMEOUT_MAXIMUM_SECONDS,
            )?,
            app_server_history_read_timeout: strict_bounded_duration_seconds(
                env,
                "DISCORD_APP_SERVER_HISTORY_READ_TIMEOUT_SECONDS",
                APP_SERVER_TIMEOUT_DEFAULT_SECONDS,
                APP_SERVER_TIMEOUT_MINIMUM_SECONDS,
                APP_SERVER_TIMEOUT_MAXIMUM_SECONDS,
            )?,
        })
    }
}

pub fn merge_env_text(env: &mut BTreeMap<String, String>, text: &str) {
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        if key.is_empty() || env.contains_key(key) {
            continue;
        }
        let value = raw_value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_owned();
        env.insert(key.to_owned(), value);
    }
}

fn required<'a>(
    env: &'a BTreeMap<String, String>,
    name: &'static str,
) -> Result<&'a str, ConfigError> {
    env.get(name)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::MissingRequired(name))
}

fn flag(env: &BTreeMap<String, String>, name: &str, default: bool) -> bool {
    let Some(raw) = env.get(name).map(String::as_str).map(str::trim) else {
        return default;
    };
    if raw.is_empty() {
        return default;
    }
    !FALSE_VALUES.contains(&raw.to_ascii_lowercase().as_str())
}

fn int_set(env: &BTreeMap<String, String>, name: &str) -> BTreeSet<u64> {
    env.get(name)
        .into_iter()
        .flat_map(|raw| raw.split(','))
        .filter_map(|part| part.trim().parse().ok())
        .collect()
}

fn optional_integer(
    env: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<u64>, ConfigError> {
    let Some(raw) = env.get(name).map(String::as_str).map(str::trim) else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse()
        .map(Some)
        .map_err(|_| ConfigError::InvalidInteger(name))
}

fn sole_value(values: &BTreeSet<u64>) -> Option<u64> {
    if values.len() == 1 {
        values.first().copied()
    } else {
        None
    }
}

fn bounded_integer(
    env: &BTreeMap<String, String>,
    name: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> u64 {
    env.get(name)
        .map(String::as_str)
        .map(str::trim)
        .and_then(|raw| raw.parse::<i128>().ok())
        .map_or(default, |value| {
            u64::try_from(value.clamp(i128::from(minimum), i128::from(maximum))).unwrap_or(default)
        })
}

fn optional_bounded_duration(
    env: &BTreeMap<String, String>,
    name: &str,
    default_seconds: f64,
    maximum_seconds: f64,
) -> Option<Duration> {
    let seconds = env
        .get(name)
        .map(String::as_str)
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .and_then(|raw| raw.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(default_seconds)
        .clamp(0.0, maximum_seconds);
    (seconds > 0.0).then(|| Duration::from_secs_f64(seconds).max(Duration::from_nanos(1)))
}

fn strict_bounded_duration_seconds(
    env: &BTreeMap<String, String>,
    name: &'static str,
    default_seconds: u64,
    minimum_seconds: u64,
    maximum_seconds: u64,
) -> Result<Duration, ConfigError> {
    let Some(raw) = env.get(name).map(String::as_str).map(str::trim) else {
        return Ok(Duration::from_secs(default_seconds));
    };
    if raw.is_empty() {
        return Ok(Duration::from_secs(default_seconds));
    }
    let seconds = raw
        .parse::<u64>()
        .map_err(|_| ConfigError::InvalidDurationSeconds(name))?;
    if !(minimum_seconds..=maximum_seconds).contains(&seconds) {
        return Err(ConfigError::DurationSecondsOutOfRange {
            name,
            minimum: minimum_seconds,
            maximum: maximum_seconds,
        });
    }
    Ok(Duration::from_secs(seconds))
}
