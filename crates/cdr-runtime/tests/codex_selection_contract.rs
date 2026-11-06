#![cfg(windows)]

use std::collections::BTreeMap;
use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use cdr_runtime::runtime_paths::{PathInputs, RuntimePaths};

fn bundle(root: &Path, label: &str, version: &str, host: bool, stamp: u64) -> PathBuf {
    let exe = root.join("OpenAI/Codex/bin").join(label).join("codex.exe");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), &exe).unwrap();
    fs::write(exe.with_file_name("probe-version.txt"), version).unwrap();
    if host {
        fs::write(
            exe.with_file_name("codex-code-mode-host.exe"),
            b"fixture host",
        )
        .unwrap();
    }
    File::options()
        .write(true)
        .open(&exe)
        .unwrap()
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(stamp)))
        .unwrap();
    exe
}

fn inputs(root: &Path, candidates: Vec<PathBuf>) -> PathInputs {
    PathInputs::new(root.to_owned(), root.join("home"), candidates, Vec::new())
}

#[test]
fn pinned_incomplete_bundle_never_runs_a_different_installation() {
    let root = tempfile::tempdir().unwrap();
    let pin = bundle(root.path(), "pin", "0.164.0", false, 300);
    let other = bundle(root.path(), "other", "0.166.0", true, 100);
    let environment = BTreeMap::from([("CODEX_EXE".into(), pin.display().to_string())]);
    let result = RuntimePaths::resolve(&environment, &inputs(root.path(), vec![other]));
    assert!(
        result.is_err(),
        "a failed explicit pin must not select another installation"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("codex-code-mode-host.exe")
    );
}

#[test]
fn installed_semantic_version_wins_over_modification_time() {
    let root = tempfile::tempdir().unwrap();
    let old = bundle(root.path(), "old", "0.164.0", true, 300);
    let latest = bundle(root.path(), "latest", "0.166.0-alpha.10", true, 100);
    let previous = bundle(root.path(), "previous", "0.166.0-alpha.9", true, 200);
    let paths = RuntimePaths::resolve(
        &BTreeMap::new(),
        &inputs(root.path(), vec![old, latest.clone(), previous]),
    )
    .unwrap();
    assert_eq!(paths.codex_exe, latest);
}

#[test]
fn incomplete_highest_version_does_not_silently_downgrade() {
    let root = tempfile::tempdir().unwrap();
    let newest = bundle(root.path(), "newest", "0.166.0", false, 300);
    let older = bundle(root.path(), "older", "0.164.0", true, 100);
    assert!(
        RuntimePaths::resolve(&BTreeMap::new(), &inputs(root.path(), vec![newest, older])).is_err()
    );
}

#[test]
fn a_complete_copy_of_the_same_highest_version_is_usable() {
    let root = tempfile::tempdir().unwrap();
    let incomplete = bundle(root.path(), "partial", "0.166.0-alpha.6", false, 300);
    let complete = bundle(root.path(), "complete", "0.166.0-alpha.6", true, 100);
    let paths = RuntimePaths::resolve(
        &BTreeMap::new(),
        &inputs(root.path(), vec![incomplete, complete.clone()]),
    )
    .unwrap();
    assert_eq!(paths.codex_exe, complete);
}

#[test]
fn an_unknown_candidate_version_cannot_be_assumed_older() {
    let root = tempfile::tempdir().unwrap();
    let unknown = bundle(root.path(), "unknown", "not-a-version", true, 300);
    let complete = bundle(root.path(), "complete", "0.166.0", true, 100);
    assert!(
        RuntimePaths::resolve(
            &BTreeMap::new(),
            &inputs(root.path(), vec![unknown, complete])
        )
        .is_err()
    );
}

#[test]
fn host_replacement_is_rejected_and_validated_files_cannot_change_during_launch() {
    let root = tempfile::tempdir().unwrap();
    let exe = bundle(root.path(), "complete", "0.166.0", true, 100);
    let paths =
        RuntimePaths::resolve(&BTreeMap::new(), &inputs(root.path(), vec![exe.clone()])).unwrap();
    let selection = paths.codex_selection.as_ref().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let lease = selection.lease(deadline).unwrap();
    assert!(fs::write(&exe, b"changed executable").is_err());
    let host = exe.with_file_name("codex-code-mode-host.exe");
    assert!(fs::write(&host, b"changed host").is_err());
    drop(lease);
    fs::write(&host, b"changed host").unwrap();
    assert!(
        selection
            .lease(deadline)
            .unwrap_err()
            .to_string()
            .contains("artifact_changed")
    );
}

#[test]
fn bounded_worker_transfers_validated_leases_without_a_gap_before_launch() {
    let root = tempfile::tempdir().unwrap();
    let exe = bundle(root.path(), "transfer", "0.166.0", true, 100);
    let paths =
        RuntimePaths::resolve(&BTreeMap::new(), &inputs(root.path(), vec![exe.clone()])).unwrap();
    let selected = paths.codex_selection.as_ref().unwrap();
    let lease = cdr_runtime::runtime_paths::selection_worker::lease_using(
        Path::new(env!("CARGO_BIN_EXE_cdr-runtime")),
        selected,
        root.path(),
        std::time::Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(lease.len(), selected.artifacts.len());
    assert!(fs::write(&exe, b"replacement").is_err());
    let host = exe.with_file_name("codex-code-mode-host.exe");
    assert!(fs::write(&host, b"replacement").is_err());
    drop(lease);
    fs::write(&host, b"replacement").unwrap();
    assert!(
        cdr_runtime::runtime_paths::selection_worker::lease_using(
            Path::new(env!("CARGO_BIN_EXE_cdr-runtime")),
            selected,
            root.path(),
            std::time::Instant::now() + Duration::from_secs(10)
        )
        .is_err()
    );
    fs::write(&host, b"locks released on error").unwrap();
}

fn junction(link: &Path, target: &Path) {
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:CDR_TEST_LINK -Target $env:CDR_TEST_TARGET -ErrorAction Stop | Out-Null"])
        .env("CDR_TEST_LINK", link).env("CDR_TEST_TARGET", target)
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_selected_plugin_image(
    root: &Path,
    selection: &cdr_runtime::runtime_paths::selection::CodexSelection,
    expected: &Path,
) {
    let log = root.join("executed-image.txt");
    fs::write(root.join("plugins.json"), "[]").unwrap();
    let mut environment = std::env::vars().collect::<std::collections::HashMap<_, _>>();
    environment.insert(
        "CDR_CODEX_CLI_FIXTURE_ROOT".into(),
        root.display().to_string(),
    );
    environment.insert("CDR_CLI_EXECUTABLE_LOG".into(), log.display().to_string());
    let output = cdr_windows_native::capture_bounded(
        Path::new(env!("CARGO_BIN_EXE_cdr-runtime")),
        &[
            "--admin".into(),
            "codex-selected-command".into(),
            "--selection-json".into(),
            serde_json::to_string(selection).unwrap(),
            "--arguments-json".into(),
            r#"["plugin","list","--json"]"#.into(),
            "--codex-home".into(),
            root.display().to_string(),
            "--repo-root".into(),
            root.display().to_string(),
        ],
        root,
        &environment,
        std::time::Instant::now() + Duration::from_secs(10),
        Duration::from_secs(1),
        65_536,
    )
    .unwrap();
    assert_eq!(
        output.code,
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["code"], 0);
    assert!(cdr_windows_native::same_resolved_path(
        Path::new(fs::read_to_string(log).unwrap().trim()),
        expected,
    ));
}

#[tokio::test]
async fn junction_retarget_after_handoff_cannot_change_the_spawned_image_or_host() {
    let root = tempfile::tempdir().unwrap();
    let first = bundle(root.path(), "first", "0.166.0", true, 100);
    let second = bundle(root.path(), "second", "0.167.0", true, 200);
    let link = root.path().join("current");
    junction(&link, first.parent().unwrap());
    let environment = BTreeMap::from([(
        "CODEX_EXE".into(),
        link.join("codex.exe").display().to_string(),
    )]);
    let paths = RuntimePaths::resolve(&environment, &inputs(root.path(), vec![])).unwrap();
    let selected = paths.codex_selection.as_ref().unwrap();
    assert!(cdr_windows_native::same_resolved_path(
        &selected.path,
        &first
    ));
    let files = cdr_runtime::runtime_paths::selection_worker::lease_using(
        Path::new(env!("CARGO_BIN_EXE_cdr-runtime")),
        selected,
        root.path(),
        std::time::Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(fs::read_link(&link).unwrap(), first.parent().unwrap());
    // Remove only this owned junction, never its target directory.
    fs::remove_dir(&link).unwrap();
    junction(&link, second.parent().unwrap());
    assert_selected_plugin_image(root.path(), selected, &first);
    let mut config = cdr_app_server::AppServerConfig::new(&selected.path);
    config.retained_executable_files = files;
    config.startup_deadline = Some(std::time::Instant::now() + Duration::from_secs(15));
    let client = cdr_app_server::AppServerClient::start(config)
        .await
        .unwrap();
    let process = cdr_windows_native::query_live_process_metadata(
        client.lifecycle_snapshot().process_id.unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(cdr_windows_native::same_resolved_path(
        &process.executable_path,
        &first
    ));
    assert!(!cdr_windows_native::same_resolved_path(
        &process.executable_path,
        &second
    ));
    assert!(
        fs::write(
            first.with_file_name("codex-code-mode-host.exe"),
            b"replacement"
        )
        .is_err()
    );
    fs::write(
        second.with_file_name("codex-code-mode-host.exe"),
        b"unrelated host",
    )
    .unwrap();
    client.close().await.unwrap();
    fs::remove_dir(link).unwrap();
}
