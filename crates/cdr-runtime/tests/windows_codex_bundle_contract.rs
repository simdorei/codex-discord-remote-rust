#![cfg(windows)]

use std::collections::BTreeMap;
use std::fs::{self, File, FileTimes};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use cdr_runtime::runtime_paths::{PathInputs, PathSource, RuntimePathError, RuntimePaths};

#[test]
fn managed_windows_codex_uses_a_complete_fallback_without_mixing_bundle_files() {
    for source in [
        PathSource::Environment,
        PathSource::LocalAppBin,
        PathSource::SandboxBin,
        PathSource::Path,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("사용자 폴더");
        let selected = if source == PathSource::SandboxBin {
            home.join(".codex/.sandbox-bin/codex.exe")
        } else {
            home.join("AppData/Local/OpenAI/Codex/bin/broken/codex.exe")
        };
        inert_executable(&selected, false, 200);
        let alternate = home.join("AppData/Local/OpenAI/Codex/bin/complete/codex.exe");
        inert_executable(&alternate, true, 100);
        let environment = if source == PathSource::Environment {
            BTreeMap::from([("CODEX_EXE".into(), selected.to_string_lossy().into_owned())])
        } else {
            BTreeMap::new()
        };
        let inputs = PathInputs::new(
            temp.path().to_owned(),
            home,
            match source {
                PathSource::Environment => vec![alternate.clone()],
                PathSource::LocalAppBin => vec![alternate.clone(), selected.clone()],
                _ => vec![],
            },
            if source == PathSource::Path {
                vec![selected.clone(), alternate.clone()]
            } else {
                vec![alternate.clone()]
            },
        );
        let missing = selected.with_file_name("codex-code-mode-host.exe");
        let resolved = RuntimePaths::resolve(&environment, &inputs).unwrap();
        assert_eq!(resolved.codex_exe, alternate);
        assert_eq!(
            resolved.codex_exe_source,
            match source {
                PathSource::Environment | PathSource::LocalAppBin => PathSource::LocalAppBin,
                PathSource::SandboxBin | PathSource::Path => PathSource::Path,
            }
        );
        assert!(!missing.exists());

        fs::write(&missing, b"matching host fixture").unwrap();
        let resolved = RuntimePaths::resolve(&environment, &inputs).unwrap();
        assert_eq!(resolved.codex_exe, selected);
        assert_eq!(resolved.codex_exe_source, source);
    }
}

fn inert_executable(path: &Path, with_host: bool, stamp: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"inert binary; must never execute").unwrap();
    if with_host {
        fs::write(
            path.with_file_name("codex-code-mode-host.exe"),
            b"inert host",
        )
        .unwrap();
    }
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(stamp)))
        .unwrap();
}

#[test]
fn automatic_discovery_skips_incomplete_app_sandbox_and_path_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let app = home.join("AppData/Local/OpenAI/Codex/bin/broken/codex.exe");
    let sandbox = home.join(".codex/.sandbox-bin/codex.exe");
    let broken_path = temp.path().join("OpenAI/Codex/bin/path-broken/codex.exe");
    let complete_path = temp.path().join("OpenAI/Codex/bin/path-complete/codex.exe");
    let alias = temp.path().join("WindowsApps/codex.exe");
    for path in [&app, &sandbox, &broken_path, &alias] {
        inert_executable(path, false, 200);
    }
    inert_executable(&complete_path, true, 100);
    let environment = BTreeMap::from([("CODEX_EXE".into(), "  \"\"  ".into())]);
    let paths = RuntimePaths::resolve(
        &environment,
        &PathInputs::new(
            temp.path().to_owned(),
            home,
            vec![app],
            vec![alias, broken_path, complete_path.clone()],
        ),
    )
    .unwrap();
    assert_eq!(paths.codex_exe, complete_path);
    assert_eq!(paths.codex_exe_source, PathSource::Path);
}

#[test]
fn no_usable_fallback_reports_the_original_missing_host() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let selected = temp.path().join("OpenAI/Codex/bin/configured/codex.exe");
    let app = home.join("AppData/Local/OpenAI/Codex/bin/broken/codex.exe");
    let sandbox = home.join(".codex/.sandbox-bin/codex.exe");
    let alias = temp.path().join("WindowsApps/codex.exe");
    for path in [&selected, &app, &sandbox, &alias] {
        inert_executable(path, false, 200);
    }
    let environment =
        BTreeMap::from([("CODEX_EXE".into(), selected.to_string_lossy().into_owned())]);
    let error = RuntimePaths::resolve(
        &environment,
        &PathInputs::new(temp.path().to_owned(), home, vec![app], vec![alias]),
    )
    .unwrap_err();
    assert_eq!(
        error,
        RuntimePathError::CodeModeHostMissing(selected.with_file_name("codex-code-mode-host.exe"))
    );
}

#[test]
fn admin_discovery_reports_missing_host_before_executing_codex() {
    let temp = tempfile::tempdir().unwrap();
    let selected = temp.path().join("OpenAI/Codex/bin/build/codex.exe");
    fs::create_dir_all(selected.parent().unwrap()).unwrap();
    fs::write(&selected, b"inert binary; must not execute").unwrap();
    let environment = format!(
        "CODEX_EXE={}\nDISCORD_BOT_TOKEN=private-fixture-token\n",
        selected.display()
    );
    fs::write(temp.path().join(".env"), &environment).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--admin", "discover-codex", "--repo-root"])
        .arg(temp.path())
        .env_clear()
        .env("USERPROFILE", temp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("codex-code-mode-host.exe"));
    assert!(!diagnostic.contains("private-fixture-token"));
    assert_eq!(
        fs::read_to_string(temp.path().join(".env")).unwrap(),
        environment
    );
}

#[test]
fn admin_discovery_falls_back_from_a_pinned_incomplete_bundle_without_changing_env() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let selected = local.join("OpenAI/Codex/bin/old/codex.exe");
    let complete = local.join("OpenAI/Codex/bin/current/codex.exe");
    inert_executable(&selected, false, 200);
    inert_executable(&complete, true, 100);
    let environment = format!(
        "CODEX_EXE={}\nDISCORD_BOT_TOKEN=private-fixture-token\n",
        selected.display()
    );
    fs::write(temp.path().join(".env"), &environment).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--admin", "discover-codex", "--repo-root"])
        .arg(temp.path())
        .env_clear()
        .env("USERPROFILE", temp.path())
        .env("LOCALAPPDATA", local)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let discovered = String::from_utf8(output.stdout).unwrap();
    assert_eq!(Path::new(discovered.trim()), complete.as_path());
    assert!(output.stderr.is_empty());
    assert!(!selected.with_file_name("codex-code-mode-host.exe").exists());
    assert_eq!(
        fs::read_to_string(temp.path().join(".env")).unwrap(),
        environment
    );
}
