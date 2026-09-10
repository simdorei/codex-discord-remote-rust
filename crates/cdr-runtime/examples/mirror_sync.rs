//! Run the same mirror synchronization as !mirror sync, without starting a second bot.
use cdr_runtime::{
    config::RuntimeConfig,
    mirror_sync::{DiscordMirrorTransport, MirrorSynchronizer},
    runtime_paths::{RuntimePaths, discover_inputs},
    startup::{StartupArgs, load_environment},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 3 || args[0] != "--apply" {
        return Err("usage: mirror_sync --apply <env-path> <origin-channel-id> [limit]".into());
    }
    let origin = args[2].parse::<u64>()?;
    let limit = args.get(3).map(|value| value.parse::<i64>()).transpose()?;
    let startup = StartupArgs::parse(["--env".into(), args[1].clone().into()])?;
    let environment = load_environment(&startup, &std::env::current_exe()?)?;
    let config = RuntimeConfig::from_map(&environment, startup.cli)?;
    let root = std::env::current_dir()?;
    let inputs = discover_inputs(&environment, root)?;
    let paths = RuntimePaths::resolve(&environment, &inputs)?;
    let http = Arc::new(twilight_http::Client::new(
        config.bot_token.expose().to_owned(),
    ));
    let sync = MirrorSynchronizer::new(
        paths.state_db,
        paths.mirror_db,
        Arc::new(DiscordMirrorTransport::new(http)),
        config.guild_id,
    );
    println!("{}", sync.sync(origin, limit).await?);
    Ok(())
}
