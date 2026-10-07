#![cfg(windows)]

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use cdr_runtime::runtime_paths::{PathInputs, PathSource, RuntimePathError, RuntimePaths};

#[test]
fn managed_windows_codex_requires_its_host_and_never_replaces_the_selected_binary() {
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
        fs::create_dir_all(selected.parent().unwrap()).unwrap();
        fs::write(&selected, b"inert Codex fixture").unwrap();
        let alternate = temp.path().join("alternate.exe");
        fs::write(&alternate, b"must not replace the selected Codex").unwrap();
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
                PathSource::LocalAppBin => vec![selected.clone()],
                _ => vec![],
            },
            if source == PathSource::Path {
                vec![selected.clone(), alternate]
            } else {
                vec![alternate]
            },
        );
        let missing = selected.with_file_name("codex-code-mode-host.exe");
        let error = RuntimePaths::resolve(&environment, &inputs).unwrap_err();
        assert_eq!(
            error,
            RuntimePathError::CodeModeHostMissing(missing.clone())
        );
        assert!(error.to_string().contains("CODEX_EXE"));

        fs::write(&missing, b"matching host fixture").unwrap();
        let resolved = RuntimePaths::resolve(&environment, &inputs).unwrap();
        assert_eq!(resolved.codex_exe, selected);
        assert_eq!(resolved.codex_exe_source, source);
    }
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
