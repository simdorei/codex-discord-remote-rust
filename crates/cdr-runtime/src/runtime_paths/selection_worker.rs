//! The supported runtime entry points contain discovery and filesystem probes in a Job.
use super::{RuntimePathError, RuntimePaths, discovery, selection::failed};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
struct Request {
    root: PathBuf,
    environment: BTreeMap<String, String>,
    remaining_ms: u64,
    #[serde(default)]
    lease: Option<LeaseRequest>,
    #[serde(default)]
    settings_home: Option<PathBuf>,
    #[serde(default)]
    command: Option<SelectedCommand>,
}

#[derive(Serialize, Deserialize)]
struct SelectedCommand {
    selection: super::selection::CodexSelection,
    arguments: Vec<String>,
    home: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn execute_selected_using(
    runtime: &Path,
    selection: super::selection::CodexSelection,
    arguments: Vec<String>,
    home: PathBuf,
    root: &Path,
    deadline: Instant,
) -> Result<CommandOutput, RuntimePathError> {
    if selection.schema_version != 1
        || !selection.path.is_absolute()
        || selection
            .artifacts
            .first()
            .is_none_or(|file| file.path != selection.path)
        || arguments.first().is_none_or(|arg| arg != "plugin")
    {
        return Err(failed("invalid selected plugin command"));
    }
    let request = Request {
        root: root.to_owned(),
        environment: BTreeMap::new(),
        remaining_ms: u64::try_from(
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis(),
        )
        .unwrap_or(u64::MAX),
        lease: None,
        settings_home: None,
        command: Some(SelectedCommand {
            selection,
            arguments,
            home,
        }),
    };
    let output = invoke(runtime, &request, deadline, Duration::from_secs(1))?;
    serde_json::from_str(&output).map_err(|e| failed(format!("invalid command response: {e}")))
}

#[derive(Serialize, Deserialize)]
struct LeaseRequest {
    selection: super::selection::CodexSelection,
    mapping: String,
    parent_pid: u32,
    parent_identity: String,
}

pub fn lease_using(
    runtime: &Path,
    selection: &super::selection::CodexSelection,
    root: &Path,
    deadline: Instant,
) -> Result<Vec<std::sync::Arc<std::fs::File>>, RuntimePathError> {
    let mapping = format!(
        "Local\\cdr-codex-lease-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let mut transfer =
        cdr_windows_native::FileLeaseTransfer::new(&mapping).map_err(|e| failed(e.to_string()))?;
    let remaining_ms = u64::try_from(
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis(),
    )
    .unwrap_or(u64::MAX);
    let request = Request {
        root: root.to_owned(),
        environment: BTreeMap::new(),
        remaining_ms,
        lease: Some(LeaseRequest {
            selection: selection.clone(),
            mapping,
            parent_pid: std::process::id(),
            parent_identity: cdr_windows_native::current_process_identity()
                .map_err(|e| failed(e.to_string()))?,
        }),
        settings_home: None,
        command: None,
    };
    let output = match invoke(runtime, &request, deadline, Duration::from_secs(1)) {
        Ok(output) => output,
        Err(error) => {
            if error.to_string().contains("cleanup_unconfirmed") {
                transfer.quarantine_until_process_exit();
                return Err(failed(format!(
                    "{error}; file leases held until process exit"
                )));
            }
            return Err(error);
        }
    };
    if output.trim() != "leases_verified" {
        return Err(failed("invalid file lease response"));
    }
    let files = transfer.take_files();
    if files.len() != selection.artifacts.len() {
        return Err(failed("file lease transfer incomplete"));
    }
    Ok(files.into_iter().map(std::sync::Arc::new).collect())
}

pub fn discover(
    environment: &BTreeMap<String, String>,
    root: &Path,
    deadline: Instant,
    cleanup: Duration,
) -> Result<RuntimePaths, RuntimePathError> {
    discover_using(
        &std::env::current_exe().map_err(|e| failed(e.to_string()))?,
        environment,
        root,
        deadline,
        cleanup,
    )
}

pub fn discover_using(
    runtime: &Path,
    environment: &BTreeMap<String, String>,
    root: &Path,
    deadline: Instant,
    cleanup: Duration,
) -> Result<RuntimePaths, RuntimePathError> {
    let work_end = deadline
        .checked_sub(cleanup)
        .ok_or_else(|| failed("selection_timeout_before_spawn"))?;
    let remaining_ms = u64::try_from(
        work_end
            .saturating_duration_since(Instant::now())
            .as_millis(),
    )
    .unwrap_or(u64::MAX);
    if remaining_ms == 0 {
        return Err(failed("selection_timeout_before_spawn"));
    }
    let keys = [
        "USERPROFILE",
        "HOME",
        "LOCALAPPDATA",
        "PATH",
        "CODEX_EXE",
        "CODEX_HOME",
        "CODEX_STATE_DB",
        "CODEX_DISCORD_ROOT",
        "CODEX_DISCORD_MIRROR_DB",
        "CODEX_BRIDGE_STATE",
        "CODEX_LOG_DB",
        "CODEX_GLOBAL_STATE",
        "CODEX_SESSION_INDEX",
        "CODEX_ARCHIVED_SESSIONS_DIR",
        "CODEX_MAINTENANCE_BACKUP_ROOT",
        "DISCORD_ATTACHMENT_DOWNLOAD_DIR",
    ];
    let environment = environment
        .iter()
        .filter(|(key, _)| keys.iter().any(|name| key.eq_ignore_ascii_case(name)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let request = Request {
        root: root.to_owned(),
        environment,
        remaining_ms,
        lease: None,
        settings_home: None,
        command: None,
    };
    let output = invoke(runtime, &request, deadline, cleanup)?;
    serde_json::from_str(&output).map_err(|e| failed(format!("invalid selection response: {e}")))
}

fn invoke(
    runtime: &Path,
    request: &Request,
    deadline: Instant,
    cleanup: Duration,
) -> Result<String, RuntimePathError> {
    let mut file = tempfile::NamedTempFile::new().map_err(|e| failed(e.to_string()))?;
    serde_json::to_writer(&mut file, &request).map_err(|e| failed(e.to_string()))?;
    file.flush().map_err(|e| failed(e.to_string()))?;
    let arguments = vec![
        "--admin".into(),
        "codex-selection-worker".into(),
        "--request-file".into(),
        file.path().display().to_string(),
        "--repo-root".into(),
        request.root.display().to_string(),
    ];
    let inherited: HashMap<_, _> = std::env::vars().collect();
    let output = cdr_windows_native::capture_bounded(
        runtime,
        &arguments,
        &request.root,
        &inherited,
        deadline,
        cleanup,
        if request.command.is_some() {
            16_777_216
        } else {
            65_536
        },
    )
    .map_err(|e| failed(e.to_string()))?;
    if output.code != 0 {
        return Err(failed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| failed(format!("invalid worker output: {e}")))
}

pub fn run(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 65_536 {
        return Err("selection request too large".into());
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let maximum = if request.command.is_some() {
        115_000
    } else {
        45_000
    };
    let deadline = Instant::now() + Duration::from_millis(request.remaining_ms.min(maximum));
    if let Some(command) = request.command {
        let _files = command
            .selection
            .lease(deadline)
            .map_err(|e| e.to_string())?;
        let mut environment: HashMap<_, _> = std::env::vars().collect();
        environment.insert("CODEX_HOME".into(), command.home.display().to_string());
        let output = cdr_windows_native::capture_bounded(
            &command.selection.path,
            &command.arguments,
            &request.root,
            &environment,
            deadline,
            Duration::from_secs(1),
            2_097_152,
        )
        .map_err(|e| e.to_string())?;
        return serde_json::to_string(&CommandOutput {
            code: output.code,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
        .map_err(|e| e.to_string());
    }
    if let Some(lease) = request.lease {
        let files = lease.selection.lease(deadline).map_err(|e| e.to_string())?;
        for (index, file) in files.iter().enumerate() {
            cdr_windows_native::transfer_file_lease(
                &lease.mapping,
                lease.parent_pid,
                &lease.parent_identity,
                index,
                file,
            )
            .map_err(|e| e.to_string())?;
        }
        return Ok("leases_verified".into());
    }
    if let Some(home) = request.settings_home {
        return super::super::admin::codex_tools::settings_digest(&home);
    }
    let mut inputs = discovery::discover_inputs(&request.environment, request.root)
        .map_err(|e| e.to_string())?;
    if super::helpers::env_path(&request.environment, "CODEX_EXE")
        .map(super::helpers::clean_executable)
        .is_none_or(str::is_empty)
    {
        inputs
            .local_app_candidates
            .extend(store_packages(deadline)?);
    }
    let paths = RuntimePaths::resolve_before(&request.environment, &inputs, deadline)
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&paths).map_err(|e| e.to_string())
}

pub fn settings_identity(home: &Path, root: &Path, deadline: Instant) -> Result<String, String> {
    let request = Request {
        root: root.to_owned(),
        environment: BTreeMap::new(),
        remaining_ms: 0,
        lease: None,
        settings_home: Some(home.to_owned()),
        command: None,
    };
    invoke(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        &request,
        deadline,
        Duration::from_secs(1),
    )
    .map(|value| value.trim().to_owned())
    .map_err(|e| e.to_string())
}

fn store_packages(deadline: Instant) -> Result<Vec<PathBuf>, String> {
    let windows =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from("C:/Windows"), PathBuf::from);
    let shell = windows.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let script = "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $p=@(Get-AppxPackage -Name OpenAI.Codex -ErrorAction Stop | ForEach-Object { Join-Path $_.InstallLocation 'app\\resources\\codex.exe' }); ConvertTo-Json -InputObject $p -Compress";
    let output = cdr_windows_native::capture_bounded(
        &shell,
        &[
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script.into(),
        ],
        &windows,
        &std::env::vars().collect(),
        deadline,
        Duration::from_millis(500),
        65_536,
    )
    .map_err(|e| e.to_string())?;
    if output.code != 0 {
        return Err("Store package registration not observable; automatic selection held".into());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("invalid Store package observation: {e}"))
}
