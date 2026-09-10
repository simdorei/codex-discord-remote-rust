//! Read-only delivery verification; prints matching message IDs, never message bodies or tokens.
use cdr_runtime::{
    config::RuntimeConfig,
    startup::{StartupArgs, load_environment},
};
use twilight_model::id::Id;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 || args[2].is_empty() {
        return Err("usage: discord_delivery_probe <env-path> <channel-id> <text-to-find>".into());
    }
    let channel = Id::new_checked(args[1].parse()?).ok_or("zero channel ID")?;
    let startup = StartupArgs::parse(["--env".into(), args[0].clone().into()])?;
    let environment = load_environment(&startup, &std::env::current_exe()?)?;
    let config = RuntimeConfig::from_map(&environment, startup.cli)?;
    let http = twilight_http::Client::new(config.bot_token.expose().to_owned());
    let messages = http
        .channel_messages(channel)
        .limit(100)
        .await?
        .models()
        .await?;
    let matching = messages
        .iter()
        .filter(|message| message.author.bot && message.content.contains(&args[2]))
        .map(|message| message.id.to_string())
        .collect::<Vec<_>>();
    println!(
        "fetched={} matching_bot_messages={} message_ids={}",
        messages.len(),
        matching.len(),
        matching.join(",")
    );
    Ok(())
}
