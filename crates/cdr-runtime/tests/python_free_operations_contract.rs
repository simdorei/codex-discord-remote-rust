#![cfg(windows)]

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn prepare(name: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = if name == "watchdog" {
        repo.join("codex-discord-watchdog.ps1")
    } else {
        repo.join(format!("plugins/codex-discord-remote/scripts/{name}.ps1"))
    };
    fs::copy(source, root.path().join("entry.ps1")).unwrap();
    let parameters = match name {
        "watchdog" => {
            "[string]$RepoRoot,[switch]$DryRun,[switch]$LogHealthy,[switch]$CheckRestartReady,[int]$RestartQuietSeconds,[int]$RestartWaitTimeoutSeconds,[int]$HealthCpuPercent,[int]$HealthFreeMemoryMb,[int]$HealthHeartbeatMaxAgeSeconds,[int]$HealthHeartbeatStartupGraceSeconds,[int]$HealthBadSampleLimit"
        }
        "restart" => {
            "[string]$RepoRoot,[switch]$DryRun,[switch]$Immediate,[switch]$Deferred,[string]$ExpectedBotIdentity,[int]$DelaySeconds,[int]$QuietSeconds,[int]$WaitTimeoutSeconds"
        }
        "status" => "[string]$RepoRoot",
        _ => unreachable!(),
    };
    fs::write(root.path().join(format!("codex-discord-rust-{name}.ps1")),
        format!("param({parameters})\n$PSBoundParameters | ConvertTo-Json -Compress\nexit ([int]$env:CDR_FIXTURE_EXIT)\n")).unwrap();
    root
}

fn run(root: &Path, name: &str, mode: Option<&str>, extra: &[&str], exit: i32) -> Output {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("entry.ps1"))
        .env_remove("CODEX_DISCORD_RUNTIME")
        .env("PYTHON_EXE", root.join("no-python.exe"))
        .env("CDR_FIXTURE_EXIT", exit.to_string());
    if let Some(mode) = mode {
        command.env("CODEX_DISCORD_RUNTIME", mode);
    }
    if name != "watchdog" {
        command.arg("-RepoRoot").arg(root);
    }
    command.args(extra).output().unwrap()
}

#[test]
fn operational_wrappers_forward_all_rust_safety_inputs_and_exit_codes() {
    for (name, arguments, expected) in [
        ("status", vec![], serde_json::json!({})),
        (
            "restart",
            vec![
                "-DryRun",
                "-Immediate",
                "-Deferred",
                "-ExpectedBotIdentity",
                "123|456",
                "-DelaySeconds",
                "31",
                "-QuietSeconds",
                "33",
                "-WaitTimeoutSeconds",
                "35",
            ],
            serde_json::json!({"ExpectedBotIdentity":"123|456", "DelaySeconds":31,"QuietSeconds":33,"WaitTimeoutSeconds":35}),
        ),
        (
            "watchdog",
            vec![
                "-DryRun",
                "-LogHealthy",
                "-CheckRestartReady",
                "-RestartQuietSeconds",
                "31",
                "-RestartWaitTimeoutSeconds",
                "32",
                "-HealthCpuPercent",
                "95",
                "-HealthFreeMemoryMb",
                "768",
                "-HealthHeartbeatMaxAgeSeconds",
                "43",
                "-HealthHeartbeatStartupGraceSeconds",
                "120",
                "-HealthBadSampleLimit",
                "2",
            ],
            serde_json::json!({"RestartQuietSeconds":31,"RestartWaitTimeoutSeconds":32,"HealthCpuPercent":95,"HealthFreeMemoryMb":768,"HealthHeartbeatMaxAgeSeconds":43,"HealthHeartbeatStartupGraceSeconds":120,"HealthBadSampleLimit":2}),
        ),
    ] {
        let root = prepare(name);
        let output = run(root.path(), name, None, &arguments, 23);
        assert_eq!(
            output.status.code(),
            Some(23),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(Path::new(actual["RepoRoot"].as_str().unwrap()), root.path());
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&actual[key], value, "{name}: {key}");
        }
        for switch in arguments.iter().filter(|arg| {
            [
                "-DryRun",
                "-Immediate",
                "-Deferred",
                "-LogHealthy",
                "-CheckRestartReady",
            ]
            .contains(arg)
        }) {
            assert_eq!(actual[&switch[1..]]["IsPresent"], true, "{name}: {switch}");
        }
    }
}

#[test]
fn old_python_mode_is_rejected_without_starting_an_alternate_runtime() {
    for name in ["status", "restart", "watchdog"] {
        let root = prepare(name);
        let output = run(root.path(), name, Some("python"), &[], 0);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("only supports the Rust runtime"),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "no alternate runtime should execute"
        );
    }
}

#[test]
fn invalid_saved_mode_is_not_silently_replaced_with_rust() {
    let root = prepare("watchdog");
    fs::write(root.path().join(".codex_discord_runtime"), "unknown").unwrap();
    let output = run(root.path(), "watchdog", None, &[], 0);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown"));
    assert!(output.stdout.is_empty());
}

#[test]
fn launcher_contains_no_interpreter_execution_or_python_recovery_path() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(repo.join("codex-discord-bot.cmd")).unwrap();
    for legacy in [
        "PYTHON_EXE",
        "codex_discord_bot.py",
        ".python-portable",
        ":python_runtime",
    ] {
        assert!(!text.contains(legacy), "launcher still depends on {legacy}");
    }
    assert!(text.contains("pushd \"%SCRIPT_DIR%\""));
    assert!(text.contains("\"%RUST_BINARY%\" --env \"%ENV_FILE%\" %SCRIPT_ARGS%"));
}
