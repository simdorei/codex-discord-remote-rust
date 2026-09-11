use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use super::{args::Args, env_file};

const APPLICATION_URL: &str = "https://discord.com/api/v10/applications/@me";
const PERMISSIONS: u64 = 328_565_115_968;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupInput {
    pub token: String,
    #[serde(default)]
    pub channel_id: String,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Application {
    pub id: String,
    pub name: String,
}

pub fn invite_url(id: &str) -> Result<String, String> {
    validate_id(id)?;
    Ok(format!(
        "https://discord.com/oauth2/authorize?client_id={id}&scope=bot%20applications.commands&permissions={PERMISSIONS}"
    ))
}

pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Discord ID must contain digits only".into());
    }
    if id.parse::<u64>().ok().is_none_or(|id| id == 0) {
        return Err("Discord ID must be a positive 64-bit number".into());
    }
    Ok(())
}

pub fn parse_application(bytes: &[u8]) -> Result<Application, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "Discord application response was not valid JSON")?;
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or("Discord did not return an application id")?
        .trim();
    validate_id(id)?;
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Discord application");
    Ok(Application {
        id: id.into(),
        name: name.into(),
    })
}

pub async fn fetch_application(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<Application, String> {
    let mut response = client
        .get(url)
        .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"))
        .header(
            reqwest::header::USER_AGENT,
            "DiscordBot (https://github.com/simdorei/codex-discord-remote-rust, 1.0)",
        )
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| format!("Discord bot token check failed: {}", error.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "Discord bot token check failed with HTTP {}",
            response.status()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        format!(
            "Discord application response read failed: {}",
            error.without_url()
        )
    })? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("Discord application response exceeded 1 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_application(&bytes)
}

pub async fn configure(
    root: &Path,
    input: SetupInput,
    client: &reqwest::Client,
    url: &str,
) -> Result<String, String> {
    let token = input.token.trim();
    if token.is_empty() || token.contains(['\n', '\r', '\0']) {
        return Err("Discord bot token is empty or contains invalid control characters".into());
    }
    let channel = input.channel_id.trim();
    if !channel.is_empty() {
        validate_id(channel)?;
    }
    let application = fetch_application(client, url, token).await?;
    let path = root.join(".env");
    let existing = env_file::read(&path)?;
    let mut updates = vec![("DISCORD_BOT_TOKEN", token)];
    let mut allowed: Vec<_> = env_file::get(&existing, "DISCORD_ALLOWED_CHANNEL_IDS")
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect();
    if !channel.is_empty() && !allowed.contains(&channel) {
        allowed.push(channel);
    }
    let allowed = allowed.join(",");
    if !channel.is_empty() {
        updates.push(("DISCORD_ALLOWED_CHANNEL_IDS", &allowed));
        if env_file::get(&existing, "DISCORD_STARTUP_CHANNEL_ID").is_none_or(str::is_empty) {
            updates.push(("DISCORD_STARTUP_CHANNEL_ID", channel));
        }
    }
    env_file::update(&path, &updates)?;
    Ok(format!(
        "Discord bot token saved to: {}\nApplication: {} ({})\nInvite link:\n{}\nOpen the invite link and authorize the bot. Channel permissions and the allowlist in .env still apply.",
        path.display(),
        application.name,
        application.id,
        invite_url(&application.id)?
    ))
}

pub(super) async fn run(args: &Args, root: &Path) -> Result<String, String> {
    if args.dry_run {
        let invite = invite_url(args.value("--bot-id").unwrap_or("123456789012345678"))?;
        return Ok(format!(
            "Dry run: no token was requested and .env was not changed.\nWould save DISCORD_BOT_TOKEN to: {}\nWould ask for the Discord general channel ID for !commands.\nInvite link:\n{invite}",
            root.join(".env").display()
        ));
    }
    if !args.input_stdin && !args.input_lines {
        return Err("setup-discord requires --input-stdin or --input-lines; use the setup-discord-bot wrapper for hidden token entry".into());
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read setup input")?;
    if bytes.len() > 65_536 {
        return Err("setup input exceeded 64 KiB".into());
    }
    let input: SetupInput = if args.input_lines {
        let text = std::str::from_utf8(&bytes).map_err(|_| "setup input must be UTF-8")?;
        let lines: Vec<_> = text.lines().collect();
        if lines.len() != 2 {
            return Err("setup input must contain exactly two lines: token and channel ID".into());
        }
        SetupInput {
            token: lines[0].into(),
            channel_id: lines[1].into(),
        }
    } else {
        serde_json::from_slice(&bytes).map_err(
            |_| "setup input must be a JSON object containing token and optional channel_id",
        )?
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    configure(root, input, &client, APPLICATION_URL).await
}
