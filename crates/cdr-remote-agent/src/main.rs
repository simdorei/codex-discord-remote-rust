use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use cdr_remote_agent::config::{RemoteMcpConfig, load_remote_mcp_config};
use cdr_remote_agent::restart_handoff::RestartHandoffRuntime;
use cdr_remote_agent::runner::{ManagedRemoteAgent, RemoteAgentControl};
use cdr_remote_agent::status::{
    ExternalMcpBuildInfo, ExternalMcpStatusPublisher, RemoteAgentStatus, normalize_install_root,
    sha256_file_hex,
};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

const BUILD_INFO_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/build-info.json"));

#[derive(Debug, Default)]
struct CliArgs {
    help: bool,
    build_info: bool,
    check_config: bool,
    env_file: Option<PathBuf>,
}

struct McpRuntimePaths {
    install_root: PathBuf,
    status_file: PathBuf,
    stop_file: PathBuf,
    instance_lock_file: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, message)) => {
            if !message.is_empty() {
                eprintln!("{message}");
            }
            ExitCode::from(code)
        }
    }
}

async fn run() -> Result<(), (u8, String)> {
    let args = parse_cli(env::args().skip(1))?;
    if args.help {
        println!("{}", help_text());
        return Ok(());
    }
    if args.build_info {
        println!("{BUILD_INFO_JSON}");
        return Ok(());
    }

    let cwd = env::current_dir().map_err(|err| (1, err.to_string()))?;
    let (env_path, install_root) = resolve_env_and_root(&cwd, args.env_file.as_deref());
    let env_map = load_environment(&env_path).map_err(|err| (1, err))?;
    let config = match load_remote_mcp_config(&env_map) {
        Ok(Some(config)) => config,
        Ok(None) => {
            return Err((
                1,
                "Remote MCP is not enabled (set CODEX_REMOTE_MCP_ENABLED=1).".into(),
            ));
        }
        Err(error) => return Err((1, error.to_string())),
    };

    if args.check_config {
        println!("Configuration OK");
        return Ok(());
    }

    let paths = resolve_mcp_paths(&install_root, &config);
    if let Some(parent) = paths.instance_lock_file.parent() {
        fs::create_dir_all(parent).map_err(|err| (1, err.to_string()))?;
    }
    let _instance_guard = acquire_single_instance(&paths.instance_lock_file)?;
    if should_abort_startup_due_to_stop_intent(&install_root, &paths.stop_file)? {
        let _ = fs::remove_file(&paths.instance_lock_file);
        return Ok(());
    }

    run_standalone_agent(config, paths).await
}

fn should_abort_startup_due_to_stop_intent(
    install_root: &Path,
    stop_file: &Path,
) -> Result<bool, (u8, String)> {
    let maint_dir = install_root.join(".cdr-maintenance");
    let exit_all_path = maint_dir.join("exit-all-intent.json");
    if exit_all_path.exists() {
        let raw = fs::read_to_string(&exit_all_path).map_err(|err| {
            (
                1,
                format!("Failed to read {}: {err}", exit_all_path.display()),
            )
        })?;
        let json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
            (
                1,
                format!("Invalid JSON in {}: {err}", exit_all_path.display()),
            )
        })?;
        let desired = json
            .get("DesiredState")
            .or_else(|| json.get("desired_state"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("stopped");
        let mcp_rev = json
            .get("McpRevision")
            .or_else(|| json.get("mcp_revision"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let intent_path = maint_dir.join("mcp-intent.json");
        let current_mcp_rev = fs::read_to_string(&intent_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| {
                v.get("Revision")
                    .or_else(|| v.get("revision"))
                    .and_then(serde_json::Value::as_u64)
            })
            .unwrap_or(0);
        if desired.eq_ignore_ascii_case("stopped") && mcp_rev >= current_mcp_rev && mcp_rev > 0 {
            return Ok(true);
        }
    }
    let intent_path = maint_dir.join("mcp-intent.json");
    if intent_path.exists() {
        let raw = fs::read_to_string(&intent_path).map_err(|err| {
            (
                1,
                format!("Failed to read {}: {err}", intent_path.display()),
            )
        })?;
        let json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
            (
                1,
                format!("Invalid JSON in {}: {err}", intent_path.display()),
            )
        })?;
        let desired = json
            .get("DesiredState")
            .or_else(|| json.get("desired_state"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if desired.eq_ignore_ascii_case("stopped") {
            return Ok(true);
        }
    }
    if stop_file.exists() {
        let raw = fs::read_to_string(stop_file)
            .map_err(|err| (1, format!("Failed to read {}: {err}", stop_file.display())))?;
        let mode = raw.trim().to_ascii_lowercase();
        if mode.contains("handoff") || mode.contains("drain") {
            let _ = fs::remove_file(stop_file);
            return Ok(false);
        }
        return Ok(true);
    }
    Ok(false)
}

async fn run_standalone_agent(
    config: RemoteMcpConfig,
    paths: McpRuntimePaths,
) -> Result<(), (u8, String)> {
    let exe_path = env::current_exe().map_err(|err| (1, err.to_string()))?;
    let artifact_sha256 = sha256_file_hex(&exe_path).map_err(|err| (1, err.to_string()))?;
    let process_identity = current_process_identity_string()?;
    let build_info: ExternalMcpBuildInfo =
        serde_json::from_str(BUILD_INFO_JSON).map_err(|err| (1, err.to_string()))?;

    let publisher = ExternalMcpStatusPublisher::new(
        paths.status_file.clone(),
        &paths.install_root,
        process_identity.clone(),
        std::process::id(),
        &exe_path,
        artifact_sha256,
        build_info,
        config.device_id.clone(),
        config.bridge_fingerprint(),
    );
    let status = RemoteAgentStatus::with_external_publisher(publisher);
    let control = RemoteAgentControl::default();
    let handoff =
        RestartHandoffRuntime::system(&paths.install_root).map_err(|err| (1, err.to_string()))?;
    let agent = ManagedRemoteAgent::initialize(config, status.clone(), handoff, control.clone())
        .await
        .map_err(|err| (1, err.to_string()))?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let heartbeat_status = status.clone();
    let mut heartbeat_shutdown = shutdown_rx.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = interval.tick() => heartbeat_status.record_heartbeat(),
                changed = heartbeat_shutdown.changed() => {
                    if changed.is_err() || *heartbeat_shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    });

    let signal_status = status.clone();
    let signal_control = control.clone();
    let stop_file = paths.stop_file.clone();
    let signal_tx = shutdown_tx.clone();
    let signal_task = tokio::spawn(async move {
        watch_stop_or_ctrl_c(&stop_file, &signal_status, &signal_control).await;
        let _ = signal_tx.send(true);
    });

    let run_result = agent.run(shutdown_rx).await;
    let _ = shutdown_tx.send(true);
    signal_task.abort();
    let _ = heartbeat_task.await;
    let _ = fs::remove_file(&paths.instance_lock_file);

    run_result.map_err(|err| (1, err.to_string()))
}

async fn watch_stop_or_ctrl_c(
    stop_file: &Path,
    status: &RemoteAgentStatus,
    control: &RemoteAgentControl,
) {
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        if let Ok(raw) = fs::read_to_string(stop_file) {
            let _ = fs::remove_file(stop_file);
            let mode = raw.trim().to_ascii_lowercase();
            if mode.contains("handoff") || mode.contains("drain") {
                status.mark_draining();
                control.request_restart_handoff();
            } else {
                status.mark_shutting_down();
            }
            return;
        }
        tokio::select! {
            _ = &mut ctrl_c => {
                status.mark_shutting_down();
                return;
            }
            () = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

#[cfg(windows)]
fn current_process_identity_string() -> Result<String, (u8, String)> {
    cdr_windows_native::current_process_identity().map_err(|err| (1, err.to_string()))
}

#[cfg(not(windows))]
fn current_process_identity_string() -> Result<String, (u8, String)> {
    Ok(format!("{}|0", std::process::id()))
}

#[cfg(windows)]
fn acquire_single_instance(
    lock_file: &Path,
) -> Result<cdr_windows_native::SingleInstance, (u8, String)> {
    let mut hasher = Sha256::new();
    hasher.update(normalize_install_root(lock_file).as_bytes());
    let digest = hex::encode(hasher.finalize());
    let mutex_name = format!("Local\\CodexDiscordRemoteMcp_{}", &digest[..24]);
    let guard = cdr_windows_native::SingleInstance::acquire(&mutex_name).map_err(|err| {
        (
            1,
            format!("Standalone remote MCP instance lock failed: {err}"),
        )
    })?;
    let identity = current_process_identity_string()?;
    let _ = fs::write(lock_file, format!("{identity}\n"));
    Ok(guard)
}

#[cfg(not(windows))]
fn acquire_single_instance(lock_file: &Path) -> Result<(), (u8, String)> {
    let _ = fs::write(lock_file, format!("{}\n", std::process::id()));
    Ok(())
}

fn parse_cli(args: impl IntoIterator<Item = String>) -> Result<CliArgs, (u8, String)> {
    let mut parsed = CliArgs::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => parsed.help = true,
            "--build-info" => parsed.build_info = true,
            "--check-config" => parsed.check_config = true,
            "--env" => {
                let Some(path) = iter.next() else {
                    return Err((2, "error: --env requires a file path".into()));
                };
                parsed.env_file = Some(PathBuf::from(path));
            }
            other => {
                return Err((2, format!("error: unrecognized arguments: {other}")));
            }
        }
    }
    Ok(parsed)
}

fn resolve_env_and_root(cwd: &Path, explicit_env: Option<&Path>) -> (PathBuf, PathBuf) {
    if let Some(path) = explicit_env {
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        let root = resolved
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(cwd)
            .to_path_buf();
        (resolved, root)
    } else {
        (cwd.join(".env"), cwd.to_path_buf())
    }
}

fn resolve_mcp_paths(install_root: &Path, config: &RemoteMcpConfig) -> McpRuntimePaths {
    McpRuntimePaths {
        install_root: install_root.to_path_buf(),
        status_file: config.resolve_status_file_path(install_root),
        stop_file: config.resolve_stop_file_path(install_root),
        instance_lock_file: config.resolve_instance_lock_path(install_root),
    }
}

fn load_environment(env_file: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut environment: BTreeMap<String, String> = env::vars().collect();
    if env_file.exists() {
        let content = fs::read_to_string(env_file)
            .map_err(|err| format!("Failed to read {}: {err}", env_file.display()))?;
        merge_env_text(&mut environment, &content);
    }
    Ok(environment)
}

fn merge_env_text(environment: &mut BTreeMap<String, String>, content: &str) {
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        if key.is_empty() || environment.contains_key(key) {
            continue;
        }
        let value = raw_value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_owned();
        environment.insert(key.to_owned(), value);
    }
}

fn help_text() -> &'static str {
    "usage: cdr-remote-agent [-h] [--build-info] [--check-config] [--env PATH]\n\n\
     Standalone native Remote MCP runtime for Codex Discord Remote.\n\n\
     options:\n  \
       -h, --help      show this help message and exit\n  \
       --build-info    print build provenance JSON and exit\n  \
       --check-config  validate Remote MCP configuration and exit\n  \
       --env PATH      optional .env file path (defaults to ./.env)"
}
