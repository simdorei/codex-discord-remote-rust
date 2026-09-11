//! Binary update ticket template: replace example paths only after review.
use cdr_app_server::{AppServerClient, AppServerConfig};
use cdr_runtime::{
    restart_readiness::{RestartReadinessState, check_restart_readiness},
    runtime_instance::RuntimeInstanceGuard,
    runtime_paths::{RuntimePaths, discover_inputs},
    startup::{StartupArgs, load_environment},
};
use std::{ffi::OsString, path::Path, time::Duration};

#[path = "support/maintenance_pins.rs"]
mod pins;

const ROOT: &str = "C:/example/codex-discord-remote-rust";
const LEGACY_MUTEX_ROOT: &str = r"C:\example\codex-discord-remote-rust";
const CODEX_HOME: &str = "C:/example/codex-home";
const STATE_DB: &str = "C:/example/codex-home/state_5.sqlite";
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Preflight,
    Cleanup,
}

fn parse(arguments: &[OsString]) -> Result<Mode> {
    if arguments.len() != 3 || arguments[1] != "--env" {
        return Err("expected preflight|cleanup --env <approved env path>".into());
    }
    match arguments[0].to_str() {
        Some("preflight") => Ok(Mode::Preflight),
        Some("cleanup") => Ok(Mode::Cleanup),
        _ => Err("unsupported maintenance mode".into()),
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("maintenance_refused: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let mode = parse(&arguments)?;
    let root = std::env::current_dir()?;
    if root.canonicalize()? != std::fs::canonicalize(ROOT)? {
        return Err("maintenance root mismatch".into());
    }
    if Path::new(&arguments[2]).canonicalize()? != root.join(".env").canonicalize()? {
        return Err("maintenance environment path mismatch".into());
    }
    let args = StartupArgs::parse(arguments.into_iter().skip(1))?;
    let environment = load_environment(&args, &std::env::current_exe()?)?;
    let paths = RuntimePaths::resolve(&environment, &discover_inputs(&environment, root.clone())?)?;
    pins::verify(
        &paths,
        Path::new(LEGACY_MUTEX_ROOT),
        Path::new(CODEX_HOME),
        Path::new(STATE_DB),
        &root.join("discord_mirror.sqlite"),
    )?;
    if mode == Mode::Cleanup {
        verify_noop_cleanup(&paths.root)?;
        println!("update_only_cleanup_confirmed; database_changes=0; discord_actions=0");
        return Ok(());
    }
    preflight(&paths).await
}

async fn preflight(paths: &RuntimePaths) -> Result<()> {
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
    let result = check_restart_readiness(
        &paths.mirror_db,
        &client,
        Duration::from_secs(15),
        Duration::from_secs(8),
    )
    .await;
    let close: Result<()> =
        match tokio::time::timeout(Duration::from_secs(10), client.close()).await {
            Ok(result) => result.map_err(Into::into),
            Err(error) => Err(format!("maintenance app-server close timed out: {error}").into()),
        };
    finish_preflight(result.map_err(Into::into), close)?;
    println!("maintenance_preflight_ready; drain_ack_still_required=true");
    Ok(())
}

fn finish_preflight(result: Result<RestartReadinessState>, close: Result<()>) -> Result<()> {
    let primary = result.and_then(|state| match state {
        RestartReadinessState::Ready => Ok(()),
        RestartReadinessState::Blocked { reason } => Err(reason.into()),
    });
    match (primary, close) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(close)) => Err(close),
        (Err(primary), Err(close)) => {
            Err(format!("{primary}; app-server close also failed: {close}").into())
        }
    }
}

fn verify_noop_cleanup(root: &Path) -> Result<()> {
    let seal = std::fs::symlink_metadata(root.join(".codex_discord_bot.disabled"))?;
    if !seal.is_file() || seal.file_type().is_symlink() {
        return Err("maintenance seal required before cleanup".into());
    }
    // The guard only publishes/removes its own temporary runtime marker. No store
    // initializer, migration, transport or synchronizer exists in this branch.
    let _exclusive = RuntimeInstanceGuard::acquire(root)?;
    Ok(())
}

#[cfg(test)]
#[path = "support/runtime_update_tests.rs"]
mod tests;
