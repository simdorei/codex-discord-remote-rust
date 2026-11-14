use super::args::Args;
use crate::codex_tools::bounded_error;
use crate::local_computer::OfficialCuaSelection;
use crate::runtime_paths::RuntimePaths;
use cdr_app_server::{AppServerClient, AppServerError};
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(super) async fn run(args: &Args, root: &Path) -> Result<String, String> {
    let official_only = matches!(
        args.command.as_str(),
        "diagnose-official-computer" | "diagnose-bundled-computer"
    );
    let deprecated_alias = args.command == "diagnose-bundled-computer";
    let invoke = probes_requested(args)?;
    let end = Instant::now() + Duration::from_mins(2);
    let DiagnosticSession {
        paths,
        computer_enabled,
        official_selection,
        config_before,
        client,
    } = start_diagnostic(args, root).await?;
    let managed_official = official_selection.is_some();
    let identity = client.lifecycle_snapshot();
    let exact_before = process_identity(identity.process_id);
    let probe_deadline = end
        .checked_sub(Duration::from_secs(8))
        .unwrap_or_else(Instant::now);
    let result = inspect_tools(
        &client,
        root,
        official_only,
        invoke,
        managed_official,
        probe_deadline,
    )
    .await;
    let diagnostic_error = result
        .as_ref()
        .err()
        .map(|error| crate::codex_tools::bounded_error(&error.to_string()));
    let config_after = settings_identity(
        &paths.codex_home,
        root,
        end.checked_sub(Duration::from_secs(8))
            .unwrap_or_else(Instant::now),
    );
    let process_after = client.lifecycle_snapshot();
    let exact_after = process_identity(process_after.process_id);
    close_diagnostic(&client, end, diagnostic_error.as_deref()).await?;
    let observation_error = config_after
        .as_ref()
        .err()
        .map(|error| crate::codex_tools::bounded_error(error));
    let tools = if config_after.as_ref().ok() != Some(&config_before)
        || exact_before.is_none()
        || exact_before != exact_after
        || !process_after.healthy
    {
        unverified_report(
            official_only,
            "environment_or_process_changed",
            diagnostic_error.as_deref(),
        )
    } else {
        result.unwrap_or_else(|error| {
            unverified_report(
                official_only,
                "diagnostic_protocol_unverified",
                Some(&error.to_string()),
            )
        })
    };
    let report = json!({
        "schema_version":1,
        "scope":"diagnostic_process_only",
        "command":if official_only { "diagnose-official-computer" } else { "diagnose-codex-tools" },
        "deprecated_alias":if deprecated_alias { Some("diagnose-bundled-computer") } else { None },
        "observed_at":chrono::Utc::now().to_rfc3339(),
        "app_server_pid":identity.process_id,
        "process_identity":exact_before,
        "codex_home":paths.codex_home,
        "cwd":root,
        "configuration_sha256":config_before,
        "configuration_observation_error":observation_error,
        "computer_provider":{
            "enabled":computer_enabled,
            "managed_registration_enabled":computer_enabled,
            "mode":if managed_official { "official_cua_standalone" } else { "configured" },
            "source":official_selection,
            "legacy_cdr_computer":"disabled"
        },
        "policy":{"approvalPolicy":"never","sandbox":"read-only","ephemeral":true},
        "tools":tools,
        "process_closed":true
    });
    #[cfg(windows)]
    let mut report = report;
    #[cfg(windows)]
    {
        report["selection"] = json!(paths.codex_selection);
    }
    serde_json::to_string(&report).map_err(|error| bounded_error(&error.to_string()))
}

fn probes_requested(args: &Args) -> Result<bool, String> {
    match args.value("--run-probes") {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        _ => Err("--run-probes must be true or false".into()),
    }
}

struct DiagnosticSession {
    paths: RuntimePaths,
    computer_enabled: bool,
    official_selection: Option<OfficialCuaSelection>,
    config_before: String,
    client: AppServerClient,
}

async fn start_diagnostic(args: &Args, root: &Path) -> Result<DiagnosticSession, String> {
    let mut environment =
        super::project_environment(root).map_err(|error| bounded_error(&error))?;
    for (arg, key) in [("--codex-exe", "CODEX_EXE"), ("--codex-home", "CODEX_HOME")] {
        if let Some(value) = args.value(arg) {
            environment.insert(key.into(), value.into());
        }
    }
    let startup = Instant::now() + cdr_app_server::APP_SERVER_STARTUP_TIMEOUT;
    let paths = RuntimePaths::discover(&environment, root, startup, Duration::from_secs(8))
        .map_err(|error| bounded_error(&error.to_string()))?;
    let computer_enabled = crate::local_computer::enabled(&environment)
        .map_err(|error| bounded_error(&error.to_string()))?;
    let mut config = paths
        .app_server_config(startup)
        .map_err(|error| bounded_error(&error.to_string()))?;
    let official_selection =
        crate::local_computer::configure_current(&mut config, computer_enabled)
            .map_err(|error| bounded_error(&error.to_string()))?;
    let config_before = settings_identity(&paths.codex_home, root, startup)
        .map_err(|error| bounded_error(&error))?;
    let client = AppServerClient::start(config)
        .await
        .map_err(|error| bounded_error(&error.to_string()))?;
    Ok(DiagnosticSession {
        paths,
        computer_enabled,
        official_selection,
        config_before,
        client,
    })
}

async fn inspect_tools(
    client: &AppServerClient,
    root: &Path,
    official_only: bool,
    invoke: bool,
    managed_official: bool,
    probe_deadline: Instant,
) -> Result<serde_json::Value, AppServerError> {
    if official_only {
        crate::local_computer::probe::inspect_with_managed_official(
            client,
            root,
            managed_official,
            probe_deadline,
        )
        .await
    } else {
        crate::codex_tools::inspect_with_managed_official(
            client,
            root,
            invoke,
            managed_official,
            probe_deadline,
        )
        .await
        .map(|report| json!(report))
    }
}

async fn close_diagnostic(
    client: &AppServerClient,
    end: Instant,
    diagnostic_error: Option<&str>,
) -> Result<(), String> {
    let close = tokio::time::timeout(
        Duration::from_secs(8).min(end.saturating_duration_since(Instant::now())),
        client.close(),
    )
    .await;
    match close {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(cleanup_failure(&error.to_string(), diagnostic_error)),
        Err(_) => Err(cleanup_failure(
            "diagnostic close timed out",
            diagnostic_error,
        )),
    }
}

fn unverified_report(official_only: bool, reason: &str, error: Option<&str>) -> serde_json::Value {
    json!({
        "contract_id":if official_only { "official-computer-inventory-v1" } else { crate::codex_tools::CONTRACT_ID },
        "state":"UNVERIFIED",
        "native_desktop_state":"NOT_CHECKED",
        "browser_state":"NOT_CHECKED",
        "desktop_app_independence":"NOT_CHECKED",
        "reason":reason,
        "error":error.map(crate::codex_tools::bounded_error)
    })
}

fn cleanup_failure(error: &str, diagnostic_error: Option<&str>) -> String {
    crate::codex_tools::bounded_error(&format!(
        "cleanup_unconfirmed: {error}; diagnostic_error={}",
        diagnostic_error.unwrap_or("none")
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_failure_keeps_original_reason_without_claiming_native_checks() {
        let report = unverified_report(
            true,
            "diagnostic_protocol_unverified",
            Some("MCP tool discovery failed: spawn EPERM"),
        );
        assert_eq!(report["error"], "MCP tool discovery failed: spawn EPERM");
        assert_eq!(report["state"], "UNVERIFIED");
        for field in [
            "native_desktop_state",
            "browser_state",
            "desktop_app_independence",
        ] {
            assert_eq!(report[field], "NOT_CHECKED");
        }
        let long = format!("spawn EPERM: {}", "한".repeat(3_000));
        let bounded = unverified_report(true, "diagnostic_protocol_unverified", Some(&long));
        let error = bounded["error"].as_str().unwrap();
        assert!(error.starts_with("spawn EPERM: "));
        assert!(error.len() <= 2_048);
    }

    #[test]
    fn cleanup_failure_does_not_erase_the_original_diagnostic_failure() {
        let error = cleanup_failure("close timed out", Some("FrameArrived timed out"));
        assert!(error.contains("cleanup_unconfirmed: close timed out"));
        assert!(error.contains("FrameArrived timed out"));
    }
}
