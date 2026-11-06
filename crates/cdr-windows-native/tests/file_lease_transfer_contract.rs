#![cfg(windows)]
use cdr_windows_native::{FileLeaseTransfer, capture_bounded, transfer_file_lease};
use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    time::{Duration, Instant},
};

#[test]
#[ignore = "owned file-transfer fixture"]
fn transfer_child() {
    if std::env::var("CDR_LEASE_MODE").unwrap() == "late" {
        let release = std::path::PathBuf::from(std::env::var_os("CDR_LEASE_RELEASE").unwrap());
        let end = Instant::now() + Duration::from_secs(3);
        while !release.exists() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(std::env::var_os("CDR_LEASE_FILE").unwrap())
        .unwrap();
    transfer_file_lease(
        &std::env::var("CDR_LEASE_NAME").unwrap(),
        std::env::var("CDR_LEASE_PID").unwrap().parse().unwrap(),
        &std::env::var("CDR_LEASE_IDENTITY").unwrap(),
        0,
        &file,
    )
    .unwrap();
    if let Some(sent) = std::env::var_os("CDR_LEASE_SENT") {
        fs::write(sent, "published").unwrap();
    }
    if std::env::var("CDR_LEASE_MODE").unwrap() == "hang" {
        std::thread::sleep(Duration::from_mins(1));
    }
}

#[test]
#[ignore = "owned unconfirmed handoff parent"]
fn quarantine_parent() {
    let path = std::path::PathBuf::from(std::env::var_os("CDR_LEASE_FILE").unwrap());
    let root = path.parent().unwrap();
    let name = format!("Local\\cdr-codex-lease-{}", uuid::Uuid::new_v4());
    let transfer = FileLeaseTransfer::new(&name).unwrap();
    let release = root.join("release");
    let sent = root.join("sent");
    let mut environment: std::collections::HashMap<_, _> = std::env::vars().collect();
    environment.extend([
        ("CDR_LEASE_NAME".into(), name),
        ("CDR_LEASE_PID".into(), std::process::id().to_string()),
        (
            "CDR_LEASE_IDENTITY".into(),
            cdr_windows_native::current_process_identity().unwrap(),
        ),
        ("CDR_LEASE_MODE".into(), "late".into()),
        ("CDR_LEASE_RELEASE".into(), release.display().to_string()),
        ("CDR_LEASE_SENT".into(), sent.display().to_string()),
    ]);
    let mut child = cdr_windows_native::CapturedWindowProcess::launch(
        &std::env::current_exe().unwrap(),
        &[
            "--exact".into(),
            "transfer_child".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        root,
        &environment,
    )
    .unwrap();
    // Inject the unconfirmed-exit state, then permit an actual late kernel duplication.
    transfer.quarantine_until_process_exit();
    fs::write(release, "continue").unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while !sent.exists() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    child.terminate_tree(Duration::from_secs(1)).unwrap();
    assert!(fs::write(&path, "still owned by parent").is_err());
    println!("quarantine_retained");
}

#[test]
fn unconfirmed_handoff_preserves_late_handles_until_owner_exit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    fs::write(&path, "original").unwrap();
    let mut environment: std::collections::HashMap<_, _> = std::env::vars().collect();
    environment.insert("CDR_LEASE_FILE".into(), path.display().to_string());
    let output = capture_bounded(
        &std::env::current_exe().unwrap(),
        &[
            "--exact".into(),
            "quarantine_parent".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        root.path(),
        &environment,
        Instant::now() + Duration::from_secs(8),
        Duration::from_secs(1),
        65_536,
    )
    .unwrap();
    assert_eq!(output.code, 0);
    assert!(String::from_utf8_lossy(&output.stdout).contains("quarantine_retained"));
    fs::write(path, "released by process exit").unwrap();
}

#[test]
fn transferred_lock_survives_helper_exit_and_partial_failure_is_reclaimable() {
    for mode in ["exit", "hang"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("artifact");
        fs::write(&path, b"original").unwrap();
        let name = format!("Local\\cdr-codex-lease-{}", uuid::Uuid::new_v4());
        let mut transfer = FileLeaseTransfer::new(&name).unwrap();
        assert!(FileLeaseTransfer::new(&name).is_err());
        let mut env: std::collections::HashMap<_, _> = std::env::vars().collect();
        env.extend([
            ("CDR_LEASE_FILE".into(), path.display().to_string()),
            ("CDR_LEASE_NAME".into(), name),
            ("CDR_LEASE_PID".into(), std::process::id().to_string()),
            (
                "CDR_LEASE_IDENTITY".into(),
                cdr_windows_native::current_process_identity().unwrap(),
            ),
            ("CDR_LEASE_MODE".into(), mode.into()),
        ]);
        let result = capture_bounded(
            &std::env::current_exe().unwrap(),
            &[
                "--exact".into(),
                "transfer_child".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            root.path(),
            &env,
            Instant::now() + Duration::from_secs(3),
            Duration::from_secs(1),
            65_536,
        );
        assert_eq!(result.is_ok(), mode == "exit");
        assert!(fs::write(&path, b"must be locked").is_err());
        if mode == "exit" {
            let files = transfer.take_files();
            assert_eq!(files.len(), 1);
            assert!(fs::write(&path, b"still locked").is_err());
            drop(files);
        }
        drop(transfer);
        fs::write(&path, b"released").unwrap();
    }
}
