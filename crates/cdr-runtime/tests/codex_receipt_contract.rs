#![cfg(windows)]

use cdr_app_server::{AppServerConfig, ResidentAppServer};
use cdr_runtime::runtime_paths::{PathInputs, RuntimePaths, receipt};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const RECEIPT: &str = ".codex_discord_rust.codex-selection.json";

async fn initialized(
    root: &Path,
    deadline: Instant,
) -> (RuntimePaths, Arc<ResidentAppServer>, u32) {
    let exe = root.join("codex.exe");
    fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), &exe).unwrap();
    fs::write(root.join("codex-code-mode-host.exe"), "fixture host").unwrap();
    let paths = RuntimePaths::resolve(
        &BTreeMap::from([("CODEX_EXE".into(), exe.display().to_string())]),
        &PathInputs::new(root.to_owned(), root.join("home"), vec![], vec![]),
    )
    .unwrap();
    let mut config = AppServerConfig::new(&paths.codex_exe);
    config.retained_executable_files = paths
        .codex_selection
        .as_ref()
        .unwrap()
        .lease(deadline)
        .unwrap();
    config.startup_deadline = Some(deadline);
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    let snapshot = server.lifecycle_snapshot().await;
    assert!(
        snapshot.healthy,
        "the receipt transaction runs after initialize"
    );
    (paths, server, snapshot.process_id.unwrap())
}

async fn assert_closed(server: &ResidentAppServer, pid: u32) {
    assert!(!server.lifecycle_snapshot().await.healthy);
    assert!(
        cdr_windows_native::query_live_process_metadata(pid)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn receipt_worker_publishes_the_initialized_identity_without_ending_the_server() {
    let root = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let (mut paths, server, pid) = initialized(root.path(), deadline).await;
    // OS path spelling is compared without a second filesystem canonicalization.
    paths.codex_exe = format!("\\\\?\\{}", paths.codex_exe.display()).into();
    receipt::write_using(
        Path::new(env!("CARGO_BIN_EXE_cdr-runtime")),
        &paths,
        &server,
        deadline,
    )
    .await
    .unwrap();
    let saved: Value =
        serde_json::from_slice(&fs::read(root.path().join(RECEIPT)).unwrap()).unwrap();
    assert_eq!(saved["app_server_pid"], pid);
    assert_eq!(saved["initialize_completed"], true);
    assert_eq!(saved["schema_version"], 1);
    assert_eq!(
        saved["app_server_process_identity"],
        cdr_windows_native::query_live_process_metadata(pid)
            .unwrap()
            .unwrap()
            .process_identity
    );
    assert!(server.lifecycle_snapshot().await.healthy);
    server.close().await.unwrap();
    assert_closed(&server, pid).await;
}

#[tokio::test]
async fn stalled_receipt_after_initialize_is_bounded_and_reaps_both_owned_processes() {
    let root = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(12);
    let (paths, server, pid) = initialized(root.path(), deadline).await;
    let error = receipt::write_using(
        Path::new(env!("CARGO_BIN_EXE_cdr-offline-soak")),
        &paths,
        &server,
        deadline,
    )
    .await
    .unwrap_err();
    assert!(
        Instant::now() < deadline,
        "cleanup is part of the common deadline: {error}"
    );
    assert!(error.to_string().contains("probe_timeout"), "{error}");
    assert!(
        !error.to_string().contains("cleanup_unconfirmed"),
        "{error}"
    );
    assert!(!root.path().join(RECEIPT).exists());
    let worker: Value =
        serde_json::from_slice(&fs::read(root.path().join("receipt-worker.json")).unwrap())
            .unwrap();
    let worker_pid = u32::try_from(worker["pid"].as_u64().unwrap()).unwrap();
    assert!(
        cdr_windows_native::query_live_process_metadata(worker_pid)
            .unwrap()
            .is_none()
    );
    fs::write(
        root.path().join("receipt-worker-lock"),
        "released after Job cleanup",
    )
    .unwrap();
    assert_closed(&server, pid).await;
}

#[tokio::test]
async fn failed_atomic_publication_does_not_report_success_and_closes_initialized_server() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(RECEIPT)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let (paths, server, pid) = initialized(root.path(), deadline).await;
    let error = receipt::write_using(
        Path::new(env!("CARGO_BIN_EXE_cdr-runtime")),
        &paths,
        &server,
        deadline,
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("receipt write unconfirmed"),
        "{error}"
    );
    assert!(root.path().join(RECEIPT).is_dir());
    assert_closed(&server, pid).await;
}

#[tokio::test]
async fn exhausted_receipt_work_budget_preserves_server_cleanup_reserve() {
    let root = tempfile::tempdir().unwrap();
    let (paths, server, pid) =
        initialized(root.path(), Instant::now() + Duration::from_secs(20)).await;
    let deadline = Instant::now() + Duration::from_secs(8);
    let error = receipt::write_using(
        Path::new(env!("CARGO_BIN_EXE_cdr-offline-soak")),
        &paths,
        &server,
        deadline,
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("probe_timeout_before_spawn"),
        "{error}"
    );
    assert!(Instant::now() < deadline);
    assert!(!root.path().join("receipt-worker.json").exists());
    assert!(!root.path().join(RECEIPT).exists());
    assert_closed(&server, pid).await;
}
