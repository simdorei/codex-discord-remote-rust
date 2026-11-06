#![cfg(windows)]
use std::fs;
use std::time::{Duration, Instant};

fn fixture(root: &std::path::Path, behavior: &str) -> std::path::PathBuf {
    let exe = root.join("codex.exe");
    fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), &exe).unwrap();
    fs::write(root.join("probe-behavior.txt"), behavior).unwrap();
    exe
}

#[test]
fn parent_exit_with_inherited_pipes_closes_the_owned_descendant() {
    let root = tempfile::tempdir().unwrap();
    let exe = fixture(root.path(), "hold-pipe");
    let begin = Instant::now();
    let output = cdr_windows_native::capture_bounded(
        &exe,
        &["--version".into()],
        root.path(),
        &std::env::vars().collect(),
        begin + Duration::from_secs(3),
        Duration::from_millis(750),
        65_536,
    )
    .unwrap();
    assert_eq!(output.code, 0);
    assert!(begin.elapsed() < Duration::from_millis(3500));
    let pid = fs::read_to_string(root.path().join("probe-child.pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        cdr_windows_native::query_live_process_metadata(pid)
            .unwrap()
            .is_none()
    );
}

#[test]
fn hung_and_flooding_probes_fail_with_bounded_collection_and_cleanup() {
    for behavior in ["hang", "flood"] {
        let root = tempfile::tempdir().unwrap();
        let exe = fixture(root.path(), behavior);
        let begin = Instant::now();
        let error = cdr_windows_native::capture_bounded(
            &exe,
            &["--version".into()],
            root.path(),
            &std::env::vars().collect(),
            begin + Duration::from_secs(3),
            Duration::from_millis(750),
            65_536,
        )
        .err()
        .expect("probe must fail");
        assert!(begin.elapsed() < Duration::from_millis(3500));
        assert!(
            error.to_string().contains(if behavior == "hang" {
                "probe_timeout"
            } else {
                "probe_output_limit"
            }),
            "{error}"
        );
    }
}

#[test]
fn exhausted_budget_does_not_start_a_probe() {
    let root = tempfile::tempdir().unwrap();
    let exe = fixture(root.path(), "hold-pipe");
    let error = cdr_windows_native::capture_bounded(
        &exe,
        &["--version".into()],
        root.path(),
        &std::env::vars().collect(),
        Instant::now(),
        Duration::from_millis(750),
        65_536,
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("before_spawn"));
    assert!(!root.path().join("probe-child.pid").exists());
}
