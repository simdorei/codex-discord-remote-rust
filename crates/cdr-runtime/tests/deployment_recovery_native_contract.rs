#![cfg(windows)]
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fixture.exe");
    fs::write(&binary, b"not executable; hash fixture only").unwrap();
    let watchdog = root.path().join("watchdog.ps1");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/restart/deployment_watchdog.ps1"),
        &watchdog,
    )
    .unwrap();
    for name in [".codex_discord_bot.disabled", ".codex_discord_rust.stop"] {
        fs::write(root.path().join(name), "owned").unwrap();
    }
    let state = serde_json::json!({"RepoRoot":root.path(),"BinaryPath":binary,"Marker":"owned","LockPath":root.path().join("guard.lock"),"Watchdog":watchdog,"BaselineHash":format!("{:X}",Sha256::digest(fs::read(&binary).unwrap())),"CandidateHash":"unused","LogPath":root.path().join("recovery.log"),"RuntimePid":42,"RuntimeTicks":"99"});
    fs::write(
        root.path().join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    root
}
fn recover(root: &Path) -> Output {
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(repo().join("scripts/Recover-CdrDeployment.ps1"))
        .arg("-StatePath")
        .arg(root.join("state.json"))
        .env("CDR_SOURCE", repo())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("fixture recovery timed out");
        }
        thread::sleep(Duration::from_millis(25));
    }
    child.wait_with_output().unwrap()
}
fn success(out: &Output) {
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn dead_worker_recovers_and_repeat_is_safe() {
    let root = fixture();
    for _ in 0..2 {
        success(&recover(root.path()));
        for name in [".codex_discord_bot.disabled", ".codex_discord_rust.stop"] {
            assert!(!root.path().join(name).exists());
        }
        assert_eq!(
            fs::read_to_string(root.path().join("started")).unwrap(),
            "yes"
        );
    }
}
#[test]
fn foreign_marker_is_preserved() {
    let root = fixture();
    let marker = root.path().join(".codex_discord_bot.disabled");
    fs::write(&marker, "user-disabled").unwrap();
    assert!(!recover(root.path()).status.success());
    assert_eq!(fs::read_to_string(marker).unwrap(), "user-disabled");
    assert!(root.path().join(".codex_discord_rust.stop").exists());
    assert!(!root.path().join("started").exists());
}
#[test]
fn other_lock_io_error_is_not_busy_or_success() {
    let root = fixture();
    let path = root.path().join("state.json");
    let mut state: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state["LockPath"] = root
        .path()
        .join("missing-parent/guard.lock")
        .to_str()
        .unwrap()
        .into();
    fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
    let out = recover(root.path());
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("deployment_busy"));
    assert!(root.path().join(".codex_discord_bot.disabled").exists());
    assert!(!root.path().join("started").exists());
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn live_worker_fences_recovery_then_process_death_releases() {
    let root = fixture();
    let mut worker=OwnedChild(Command::new("powershell.exe").args(["-NoProfile","-Command",r"$f=[IO.File]::Open((Join-Path $env:CDR_LOCK_ROOT 'guard.lock'),'OpenOrCreate','ReadWrite','None');[IO.File]::WriteAllText((Join-Path $env:CDR_LOCK_ROOT 'ready'),'yes');[Console]::ReadLine() | Out-Null"]).env("CDR_LOCK_ROOT",root.path()).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("ready").exists() {
        assert!(
            worker.0.try_wait().unwrap().is_none(),
            "lock fixture exited"
        );
        assert!(Instant::now() < deadline, "lock fixture timeout");
        thread::sleep(Duration::from_millis(25));
    }
    let out = recover(root.path());
    assert_eq!(out.status.code(), Some(75));
    assert!(String::from_utf8_lossy(&out.stdout).contains("deployment_busy"));
    assert!(root.path().join(".codex_discord_bot.disabled").exists());
    assert!(!root.path().join("started").exists());
    // Kill only this test's handle-owning fixture; process death must release the lock.
    worker.0.stdin.as_mut().unwrap().flush().unwrap();
    drop(worker);
    success(&recover(root.path()));
    assert!(!root.path().join(".codex_discord_bot.disabled").exists());
    assert!(root.path().join("started").exists());
}
