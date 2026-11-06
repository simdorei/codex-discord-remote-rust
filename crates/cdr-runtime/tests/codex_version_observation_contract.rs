#![cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    fs,
    process::{Child, Command},
    time::{Duration, Instant},
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn version_observation_checks_exact_live_identity_and_has_a_bounded_probe() {
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("codex.exe");
    fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), &exe).unwrap();
    let mut child = OwnedChild(
        Command::new(&exe)
            .arg("--probe-child")
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap(),
    );
    let metadata = cdr_windows_native::query_live_process_metadata(child.0.id())
        .unwrap()
        .unwrap();
    let run = |identity: &str| {
        Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
            .args(["--admin", "codex-version", "--repo-root"])
            .arg(root.path())
            .args(["--pid", &child.0.id().to_string(), "--identity", identity])
            .output()
            .unwrap()
    };
    let output = run(&metadata.process_identity);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "0.166.0-alpha.6"
    );
    let output = run(&format!("{}|1", child.0.id()));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("identity changed"));
    fs::write(root.path().join("probe-behavior.txt"), "hold-pipe").unwrap();
    assert!(run(&metadata.process_identity).status.success());
    let probe_pid = fs::read_to_string(root.path().join("probe-child.pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        cdr_windows_native::query_live_process_metadata(probe_pid)
            .unwrap()
            .is_none()
    );
    fs::write(root.path().join("probe-behavior.txt"), "hang").unwrap();
    let begin = Instant::now();
    assert!(!run(&metadata.process_identity).status.success());
    assert!(begin.elapsed() < Duration::from_secs(6));
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "observation must not terminate the existing process"
    );
}
