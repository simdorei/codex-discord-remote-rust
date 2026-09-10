//! Maintenance ticket template. Replace every example value only after review.
use cdr_app_server::{AppServerClient, AppServerConfig};
use cdr_runtime::{
    config::RuntimeConfig,
    mirror_sync::{DiscordMirrorTransport, MirrorSynchronizer},
    restart_readiness::{
        RestartReadinessState,
        maintenance::{AbsentTarget, check_absent_maintenance},
    },
    runtime_instance::RuntimeInstanceGuard,
    runtime_paths::{RuntimePaths, discover_inputs},
    startup::{StartupArgs, load_environment},
};
use std::{sync::Arc, time::Duration};

const ROOT: &str = "C:/example/codex-discord-remote-rust";
const LEGACY_MUTEX_ROOT: &str = r"C:\example\codex-discord-remote-rust";
const CODEX_HOME: &str = "C:/example/codex-home";
const STATE_DB: &str = "C:/example/codex-home/state_5.sqlite";
const TARGET: &str = "00000000-0000-0000-0000-000000000000";
const ROOM: u64 = 100;
const PARENT: u64 = 200;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("maintenance_refused: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let mode = arguments.next().ok_or("expected preflight or cleanup")?;
    if mode != "preflight" && mode != "cleanup" {
        return Err("unsupported maintenance mode".into());
    }
    // Match the bot's current_dir spelling for its legacy path-hashed mutex.
    // canonicalize() adds a Windows extended-path prefix and would change its name.
    let root = std::env::current_dir()?;
    if root.canonicalize()? != std::fs::canonicalize(ROOT)? {
        return Err("maintenance root mismatch".into());
    }
    let args = StartupArgs::parse(arguments)?;
    let environment = load_environment(&args, &std::env::current_exe()?)?;
    let paths = RuntimePaths::resolve(&environment, &discover_inputs(&environment, root.clone())?)?;
    if paths.root.as_os_str() != std::ffi::OsStr::new(LEGACY_MUTEX_ROOT) {
        return Err("maintenance legacy mutex root spelling mismatch".into());
    }
    if paths.codex_home.canonicalize()? != std::fs::canonicalize(CODEX_HOME)?
        || paths.state_db.canonicalize()? != std::fs::canonicalize(STATE_DB)?
    {
        return Err("maintenance approved Codex home/state DB mismatch".into());
    }
    if paths.mirror_db.canonicalize()? != root.join("discord_mirror.sqlite").canonicalize()? {
        return Err("maintenance DB mismatch".into());
    }
    if mode == "preflight" {
        let config = AppServerConfig::new(&paths.codex_exe).with_environment([(
            "CODEX_HOME".into(),
            paths
                .codex_home
                .to_str()
                .ok_or("invalid Codex path")?
                .into(),
        )]);
        let client =
            tokio::time::timeout(Duration::from_secs(15), AppServerClient::start(config)).await??;
        let result = check_absent_maintenance(
            &paths.mirror_db,
            &client,
            Duration::from_secs(15),
            Duration::from_secs(8),
            &AbsentTarget {
                state_db: paths.state_db,
                thread_id: TARGET.into(),
                room: i64::try_from(ROOM)?,
                parent: i64::try_from(PARENT)?,
            },
        )
        .await;
        tokio::time::timeout(Duration::from_secs(10), client.close()).await??;
        match result? {
            RestartReadinessState::Ready => {
                println!("maintenance_preflight_ready; drain_ack_still_required=true");
            }
            RestartReadinessState::Blocked { reason } => return Err(reason.into()),
        }
        return Ok(());
    }
    // Must acquire the same OS single-instance lock as the bot BEFORE any store
    // initializer. A concurrently running old bot therefore prevents migration.
    if !root.join(".codex_discord_bot.disabled").is_file() {
        return Err("maintenance seal required before cleanup".into());
    }
    let _exclusive = RuntimeInstanceGuard::acquire(&paths.root)?;
    let config = RuntimeConfig::from_map(&environment, args.cli)?;
    let http = Arc::new(twilight_http::Client::new(config.bot_token.expose().into()));
    let sync = MirrorSynchronizer::new(
        paths.state_db,
        paths.mirror_db,
        Arc::new(DiscordMirrorTransport::new(http)),
        config.guild_id,
    );
    sync.retire_exact_absent(TARGET, ROOM, PARENT).await?;
    println!("exact_cleanup_completed target={TARGET} room={ROOM}");
    Ok(())
}
