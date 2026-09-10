use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

pub fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("codex-discord-memory-ab.ps1")
}

fn ps_literal(value: &str) -> String {
    value.replace('\'', "''")
}

pub struct OwnedProcessPair {
    pub parent: Child,
    pub app_pid: u32,
    pub bot_started_at_utc: String,
    pub app_started_at_utc: String,
    pub bot_executable_path: PathBuf,
    pub app_executable_path: PathBuf,
}

pub struct OwnedChild {
    child: Child,
}

impl OwnedChild {
    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn spawn_renamed_benign_runtime(root: &Path, source: &Path) -> OwnedChild {
    let executable = root.join("cdr-runtime.exe");
    fs::copy(source, &executable).expect("copy benign PowerShell under duplicate-runtime name");
    let mut child = Command::new(executable)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 60",
        ])
        .spawn()
        .expect("spawn renamed benign PowerShell");
    thread::sleep(Duration::from_millis(250));
    assert!(
        child.try_wait().unwrap().is_none(),
        "renamed benign process exited"
    );
    OwnedChild { child }
}

impl Drop for OwnedProcessPair {
    fn drop(&mut self) {
        let cleanup = r"
$p = Get-Process -Id __PID__ -ErrorAction SilentlyContinue
if ($null -ne $p) {
    $sameStart = $p.StartTime.ToUniversalTime().Ticks -eq ([DateTimeOffset]'__START__').UtcDateTime.Ticks
    $samePath = ([IO.Path]::GetFullPath([string]$p.Path)).Equals(
        '__PATH__', [StringComparison]::OrdinalIgnoreCase
    )
    if ($sameStart -and $samePath) { Stop-Process -Id $p.Id -Force }
}
"
        .replace("__PID__", &self.app_pid.to_string())
        .replace("__START__", &ps_literal(&self.app_started_at_utc))
        .replace(
            "__PATH__",
            &ps_literal(&self.app_executable_path.to_string_lossy()),
        );
        let _ = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &cleanup])
            .status();
        let _ = self.parent.kill();
        let _ = self.parent.wait();
    }
}

pub fn spawn_owned_process_pair(root: &Path) -> OwnedProcessPair {
    let metadata_path = root.join("owned-processes.json");
    let command = r"
$child = Start-Process -FilePath 'powershell.exe' -ArgumentList @(
    '-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 60'
) -PassThru
$self = Get-Process -Id $PID -ErrorAction Stop
$payload = [ordered]@{
    bot_started_at_utc = $self.StartTime.ToUniversalTime().ToString('o')
    bot_executable_path = $self.Path
    app_pid = [int]$child.Id
    app_started_at_utc = $child.StartTime.ToUniversalTime().ToString('o')
    app_executable_path = $child.Path
} | ConvertTo-Json -Compress
[IO.File]::WriteAllText('__METADATA__', $payload, [Text.UTF8Encoding]::new($false))
Start-Sleep -Seconds 60
"
    .replace(
        "__METADATA__",
        &ps_literal(&metadata_path.to_string_lossy()),
    );
    let parent = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .spawn()
        .expect("spawn benign parent PowerShell");

    let deadline = Instant::now() + Duration::from_secs(10);
    while !metadata_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    let metadata: Value = serde_json::from_slice(
        &fs::read(&metadata_path).expect("benign process metadata was not published"),
    )
    .expect("parse benign process metadata");
    OwnedProcessPair {
        parent,
        app_pid: u32::try_from(metadata["app_pid"].as_u64().unwrap()).unwrap(),
        bot_started_at_utc: metadata["bot_started_at_utc"].as_str().unwrap().into(),
        app_started_at_utc: metadata["app_started_at_utc"].as_str().unwrap().into(),
        bot_executable_path: metadata["bot_executable_path"].as_str().unwrap().into(),
        app_executable_path: metadata["app_executable_path"].as_str().unwrap().into(),
    }
}

pub struct SampleRequest<'a> {
    pub bot: &'a OwnedProcessPair,
    pub app: Option<&'a OwnedProcessPair>,
    pub authorized: bool,
    pub short_override: bool,
    pub non_discord_override: bool,
    pub bot_started_at_utc: &'a str,
    pub bot_executable_path: &'a Path,
    pub duration_seconds: &'a str,
}

impl<'a> SampleRequest<'a> {
    pub fn valid(pair: &'a OwnedProcessPair) -> Self {
        Self {
            bot: pair,
            app: Some(pair),
            authorized: true,
            short_override: true,
            non_discord_override: true,
            bot_started_at_utc: &pair.bot_started_at_utc,
            bot_executable_path: &pair.bot_executable_path,
            duration_seconds: "1.2",
        }
    }
}

pub fn run_sample(root: &Path, request: &SampleRequest<'_>) -> Output {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script_path())
        .args(["-Phase", "warm_idle", "-RuntimeLabel", "rust-test"])
        .args(["-WorkloadId", "workload-42", "-DbSnapshotId", "db-copy-7"])
        .args(["-CodexVersion", "0.146.0-test", "-BotPid"])
        .arg(request.bot.parent.id().to_string())
        .args(["-BotStartedAtUtc", request.bot_started_at_utc])
        .arg("-BotExecutablePath")
        .arg(request.bot_executable_path)
        .args(["-DurationSeconds", request.duration_seconds])
        .args(["-SampleIntervalSeconds", "0.1"])
        .arg("-RawJsonlPath")
        .arg(root.join("samples.jsonl"))
        .arg("-SummaryPath")
        .arg(root.join("summary.json"));
    if let Some(app) = request.app {
        command
            .arg("-AppServerPid")
            .arg(app.app_pid.to_string())
            .args(["-AppServerStartedAtUtc", &app.app_started_at_utc])
            .arg("-AppServerExecutablePath")
            .arg(&app.app_executable_path)
            .args(["-ReadyAtUtc", &app.app_started_at_utc]);
    } else {
        command.args(["-ReadyAtUtc", request.bot_started_at_utc]);
    }
    if request.authorized {
        command.arg("-AuthorizedLiveMeasurement");
    }
    if request.short_override {
        command.arg("-TestOnlyAllowShortDuration");
    }
    if request.non_discord_override {
        command.arg("-TestOnlyAllowNonDiscordProcesses");
    }
    command.output().unwrap()
}
