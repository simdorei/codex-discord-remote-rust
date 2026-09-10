#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn script_paths() -> [PathBuf; 7] {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    [
        root.join("codex-discord-memory-ab.ps1"),
        root.join("scripts/codex-discord-memory-ab-common.ps1"),
        root.join("scripts/codex-discord-memory-ab-process.ps1"),
        root.join("scripts/codex-discord-memory-ab-report.ps1"),
        root.join("scripts/codex-discord-memory-ab-sample.ps1"),
        root.join("scripts/codex-discord-memory-ab-validation.ps1"),
        root.join("scripts/codex-discord-memory-ab-compare.ps1"),
    ]
}

#[test]
fn script_declares_opt_in_non_mutating_pid_bound_contract() {
    let texts: Vec<String> = script_paths()
        .iter()
        .map(|path| fs::read_to_string(path).expect("read memory A/B script module"))
        .collect();
    for (path, text) in script_paths().iter().zip(&texts) {
        assert!(
            text.lines().count() <= 250,
            "{} mixes too many responsibilities",
            path.display()
        );
    }
    let text = texts.join("\n");

    for required in [
        "DefaultParameterSetName = 'SamplePhase'",
        "ParameterSetName = 'SamplePhase'",
        "ParameterSetName = 'Compare'",
        "AuthorizedLiveMeasurement",
        "BotExecutablePath",
        "AppServerExecutablePath",
        "TestOnlyAllowShortDuration",
        "TestOnlyAllowNonDiscordProcesses",
        "DurationSeconds = 300",
        "SampleIntervalSeconds = 1",
        "warm_idle",
        "active",
        "recovery",
        "Get-CimInstance",
        "ParentProcessId",
        "Assert-MemoryAbNoSecondLiveDiscordBot",
        "second live Discord bot detected",
        "cdr-runtime.exe",
        "StartTime.ToUniversalTime",
        "runtime_kind",
        "working_set_bytes",
        "private_memory_bytes",
        "cpu_one_core_percent",
        "handle_count",
        "thread_count",
        "AppendAllText",
        "FileMode]::CreateNew",
        "workload_id = $WorkloadId",
        "db_snapshot_id = $DbSnapshotId",
    ] {
        assert!(
            text.contains(required),
            "missing contract token: {required}"
        );
    }
    for forbidden in [
        "Start-Process",
        "Stop-Process",
        ".codex_discord_bot.disabled",
        "codex-discord-watchdog",
    ] {
        assert!(
            !text.contains(forbidden),
            "measurement script must not contain {forbidden}"
        );
    }
}

#[test]
fn powershell_parser_accepts_memory_ab_script() {
    for script in script_paths() {
        let command = format!(
            "$e=$null; [void][Management.Automation.Language.Parser]::ParseFile('{}',[ref]$null,[ref]$e); if ($e.Count) {{ $e | % Message; exit 1 }}",
            script.display().to_string().replace('\'', "''")
        );
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &command])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} parse failed\nstdout: {}\nstderr: {}",
            script.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
