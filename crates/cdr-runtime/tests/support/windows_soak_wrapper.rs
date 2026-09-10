#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MARKER: &[u8] = b"operator_disabled\n";

pub struct Fixture {
    pub root: PathBuf,
    pub script: PathBuf,
    pub target: PathBuf,
    pub debug_harness: PathBuf,
    pub release_harness: PathBuf,
    pub evidence: PathBuf,
    pub source_file: PathBuf,
    pub marker: PathBuf,
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn prepare(root: &Path) -> Fixture {
    let scripts = root.join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    let script = root.join("codex-discord-rust-soak.ps1");
    fs::copy(repo_root().join("codex-discord-rust-soak.ps1"), &script).unwrap();
    for module in [
        "CodexDiscordSoak.Provenance.psm1",
        "CodexDiscordSoak.ProcessOwnership.psm1",
        "CodexDiscordSoak.SourceFingerprint.psm1",
        "CodexDiscordSoak.Evidence.psm1",
    ] {
        fs::copy(
            repo_root().join("scripts").join(module),
            scripts.join(module),
        )
        .unwrap();
    }
    for module in [
        "CodexDiscordSoak.WrapperRuntime.psm1",
        "CodexDiscordSoak.WrapperEvidence.psm1",
    ] {
        let source = repo_root().join("scripts").join(module);
        if source.exists() {
            fs::copy(source, scripts.join(module)).unwrap();
        }
    }
    fs::write(root.join("Cargo.toml"), b"[workspace]\nmembers=[]\n").unwrap();
    fs::write(root.join("Cargo.lock"), b"version = 3\n").unwrap();
    fs::write(
        root.join("rust-toolchain.toml"),
        b"[toolchain]\nchannel='stable'\n",
    )
    .unwrap();
    let source_file = root.join("crates/probe/src/lib.rs");
    fs::create_dir_all(source_file.parent().unwrap()).unwrap();
    fs::write(&source_file, b"alpha").unwrap();
    let marker = root.join(".codex_discord_bot.disabled");
    fs::write(&marker, MARKER).unwrap();
    let target = root.join("target");
    let debug_harness = target.join("debug/cdr-offline-soak.exe");
    let release_harness = target.join("release/cdr-offline-soak.exe");
    fs::create_dir_all(debug_harness.parent().unwrap()).unwrap();
    fs::create_dir_all(release_harness.parent().unwrap()).unwrap();
    for harness in [&debug_harness, &release_harness] {
        fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), harness).unwrap();
    }
    Fixture {
        root: root.to_path_buf(),
        script,
        target,
        debug_harness,
        release_harness,
        evidence: root.join("evidence"),
        source_file,
        marker,
    }
}

pub fn sha256(path: &Path) -> String {
    hex::encode_upper(Sha256::digest(fs::read(path).unwrap()))
}

fn base_command(fixture: &Fixture, harness: &Path, duration: u64) -> Command {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&fixture.script)
        .arg("-RepoRoot")
        .arg(&fixture.root)
        .arg("-OutputDirectory")
        .arg(&fixture.evidence)
        .arg("-HarnessPath")
        .arg(harness)
        .args(["-ExpectedHarnessSha256", &sha256(harness)])
        .args(["-DurationSeconds", &duration.to_string()])
        .args([
            "-SampleIntervalSeconds",
            "0.1",
            "-WarmupSeconds",
            "0",
            "-HarnessExitGraceSeconds",
            "5",
            "-MaxSlopeBytesPerHour",
            "1000000000000000",
        ])
        .env("CARGO_TARGET_DIR", &fixture.target)
        .env_remove("DISCORD_BOT_TOKEN")
        .env_remove("DISCORD_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

pub fn skip_build_command(fixture: &Fixture, duration: u64) -> Command {
    let mut command = base_command(fixture, &fixture.debug_harness, duration);
    command.arg("-SkipBuild");
    command
}

pub fn build_command(fixture: &Fixture, cargo: &Path, duration: u64) -> Command {
    let mut command = base_command(fixture, &fixture.release_harness, duration);
    command.arg("-CargoPath").arg(cargo);
    command
}

pub fn fake_cargo(fixture: &Fixture, mutate: bool, exit_code: i32) -> PathBuf {
    let path = fixture
        .root
        .join(format!("fake-cargo-{mutate}-{exit_code}.ps1"));
    let mutation = if mutate {
        "[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'crates\\probe\\src\\lib.rs'), 'bravo', [Text.UTF8Encoding]::new($false))"
    } else {
        ""
    };
    fs::write(
        &path,
        format!(
            "param([Parameter(ValueFromRemainingArguments=$true)][string[]]$Rest)\n{mutation}\nexit {exit_code}\n"
        ),
    )
    .unwrap();
    path
}

pub fn wait_for_memory(mut child: Child, fixture: &Fixture) -> (Child, Value) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(record) = first_memory_record(fixture) {
            return (child, record);
        }
        if child.try_wait().unwrap().is_some() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "wrapper exited before memory evidence\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if Instant::now() >= deadline {
            terminate_tree(child.id());
            let _ = child.wait();
            panic!("timed out waiting for memory evidence");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn first_memory_record(fixture: &Fixture) -> Option<Value> {
    fs::read_dir(&fixture.evidence)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.path().to_string_lossy().ends_with(".memory.jsonl"))
        .and_then(|entry| fs::read_to_string(entry.path()).ok())
        .and_then(|text| {
            text.lines()
                .find(|line| !line.trim().is_empty())
                .map(str::to_owned)
        })
        .and_then(|line| serde_json::from_str(&line).ok())
}

pub fn wait_for_output(mut child: Child, timeout: Duration) -> Output {
    let deadline = Instant::now() + timeout;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            terminate_tree(child.id());
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "wrapper timed out\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
    child.wait_with_output().unwrap()
}

pub fn terminate_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output();
}

pub fn summary(fixture: &Fixture) -> Value {
    let path = fs::read_dir(&fixture.evidence)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.ends_with(".summary.json") && !name.ends_with(".harness.summary.json")
        })
        .expect("wrapper summary");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

pub fn assert_marker(fixture: &Fixture) {
    assert_eq!(fs::read(&fixture.marker).unwrap(), MARKER);
}
