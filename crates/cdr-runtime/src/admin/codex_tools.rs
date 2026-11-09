use super::args::Args;
use crate::runtime_paths::RuntimePaths;
use cdr_app_server::AppServerClient;
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(super) async fn run(args: &Args, root: &Path) -> Result<String, String> {
    let bundled_only = args.command == "diagnose-bundled-computer";
    let invoke = match args.value("--run-probes") {
        None | Some("false") => false,
        Some("true") => true,
        _ => return Err("--run-probes must be true or false".into()),
    };
    let end = Instant::now() + Duration::from_mins(2);
    let mut environment = super::project_environment(root)?;
    for (arg, key) in [("--codex-exe", "CODEX_EXE"), ("--codex-home", "CODEX_HOME")] {
        if let Some(value) = args.value(arg) {
            environment.insert(key.into(), value.into());
        }
    }
    let startup = Instant::now() + cdr_app_server::APP_SERVER_STARTUP_TIMEOUT;
    let paths = RuntimePaths::discover(&environment, root, startup, Duration::from_secs(8))
        .map_err(|e| e.to_string())?;
    let computer_enabled =
        crate::local_computer::enabled(&environment).map_err(|e| e.to_string())?;
    if bundled_only && !computer_enabled {
        return Err("Bundled Computer Use is disabled for this runtime.".into());
    }
    let mut config = paths
        .app_server_config(startup)
        .map_err(|e| e.to_string())?;
    crate::local_computer::configure_current(&mut config, computer_enabled)
        .map_err(|e| e.to_string())?;
    let computer_executable = if computer_enabled {
        Some(std::env::current_exe().map_err(|e| e.to_string())?)
    } else {
        None
    };
    let config_before = settings_identity(&paths.codex_home, root, startup)?;
    let client = AppServerClient::start(config)
        .await
        .map_err(|e| e.to_string())?;
    let identity = client.lifecycle_snapshot();
    let exact_before = process_identity(identity.process_id);
    let probe_deadline = end
        .checked_sub(Duration::from_secs(8))
        .unwrap_or_else(Instant::now);
    let result = if bundled_only {
        crate::local_computer::probe::inspect(&client, root, probe_deadline).await
    } else {
        crate::codex_tools::inspect(&client, root, invoke, probe_deadline)
            .await
            .map(|report| json!(report))
    };
    let config_after = settings_identity(
        &paths.codex_home,
        root,
        end.checked_sub(Duration::from_secs(8))
            .unwrap_or_else(Instant::now),
    );
    let process_after = client.lifecycle_snapshot();
    let exact_after = process_identity(process_after.process_id);
    let close = tokio::time::timeout(
        Duration::from_secs(8).min(end.saturating_duration_since(Instant::now())),
        client.close(),
    )
    .await;
    close
        .map_err(|_| "cleanup_unconfirmed: diagnostic close".to_owned())?
        .map_err(|e| e.to_string())?;
    let tools = if config_after.as_ref().ok() != Some(&config_before)
        || exact_before.is_none()
        || exact_before != exact_after
        || !process_after.healthy
    {
        json!({"contract_id":crate::codex_tools::CONTRACT_ID,"state":"UNVERIFIED","reason":"environment_or_process_changed"})
    } else {
        result.unwrap_or_else(|_|json!({"contract_id":crate::codex_tools::CONTRACT_ID,"state":"UNVERIFIED","reason":"diagnostic_protocol_unverified"}))
    };
    let report = json!({"schema_version":1,"scope":"diagnostic_process_only","observed_at":chrono::Utc::now().to_rfc3339(),"app_server_pid":identity.process_id,"process_identity":exact_before,"codex_home":paths.codex_home,"cwd":root,"configuration_sha256":config_before,"computer_provider":{"enabled":computer_enabled,"mode":"bundled_same_executable","executable":computer_executable},"policy":{"approvalPolicy":"never","sandbox":"read-only","ephemeral":true},"tools":tools,"process_closed":true});
    #[cfg(windows)]
    let mut report = report;
    #[cfg(windows)]
    {
        report["selection"] = json!(paths.codex_selection);
    }
    serde_json::to_string(&report).map_err(|e| e.to_string())
}

fn process_identity(pid: Option<u32>) -> Option<String> {
    #[cfg(windows)]
    {
        cdr_windows_native::query_live_process_metadata(pid?)
            .ok()??
            .process_identity
            .into()
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        None
    }
}

fn settings_identity(home: &Path, root: &Path, deadline: Instant) -> Result<String, String> {
    #[cfg(windows)]
    {
        crate::runtime_paths::selection_worker::settings_identity(home, root, deadline)
    }
    #[cfg(not(windows))]
    {
        let _ = (root, deadline);
        settings_digest(home)
    }
}

pub(crate) fn settings_digest(home: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut bytes = vec![];
    match std::fs::File::open(home.join("config.toml")) {
        Ok(file) => {
            file.take(1_048_577)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    if bytes.len() > 1_048_576 {
        return Err("configuration observation exceeds 1 MiB".into());
    }
    Ok(hex::encode(Sha256::digest(bytes)))
}
