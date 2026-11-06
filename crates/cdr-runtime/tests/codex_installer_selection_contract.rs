#![cfg(windows)]
use std::fs;
#[path = "support/installer_native.rs"]
mod fixture;

#[test]
fn validated_explicit_selection_ignores_a_broken_path_shim() {
    let root = tempfile::tempdir().unwrap();
    let selected = fixture::seed(root.path());
    fixture::inventories(root.path(), "normal");
    let shim = root.path().join("broken-path");
    fs::create_dir(&shim).unwrap();
    fs::write(shim.join("codex.cmd"), "@exit /b 96\r\n").unwrap();
    let output = fixture::run(root.path(), true, &selected, |command| {
        let path = std::env::var("PATH").unwrap();
        command.env("PATH", format!("{};{path}", shim.display()));
    });
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(root.path().join("cli-calls.jsonl")).unwrap();
    assert_eq!(calls.lines().count(), 4);
}

#[test]
fn incomplete_pin_preserves_environment_and_executes_zero_plugin_commands() {
    let root = tempfile::tempdir().unwrap();
    let complete = fixture::seed(root.path());
    fixture::inventories(root.path(), "normal");
    let pin = root.path().join("OpenAI/Codex/bin/pinned/codex.exe");
    fs::create_dir_all(pin.parent().unwrap()).unwrap();
    fs::copy(complete, &pin).unwrap();
    let original = fs::read(root.path().join(".env")).unwrap();
    let output = fixture::bounded(fixture::command(root.path(), true, Some(&pin), false));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("codex-code-mode-host.exe"));
    assert_eq!(fs::read(root.path().join(".env")).unwrap(), original);
    assert!(!root.path().join("cli-calls.jsonl").exists());
}

#[test]
fn explicit_empty_pin_uses_automatic_selection_before_saving_configuration() {
    for prior in ["missing", "older", "automatic-failure"] {
        let root = tempfile::tempdir().unwrap();
        let selected = fixture::seed(root.path());
        fixture::inventories(root.path(), "normal");
        let old = root.path().join("old/codex.exe");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        if prior != "missing" {
            fs::copy(&selected, &old).unwrap();
            fs::write(old.with_file_name("probe-version.txt"), "0.164.0").unwrap();
        }
        let original = format!("KEEP=preserved\nCODEX_EXE={}\n", old.display());
        fs::write(root.path().join(".env"), &original).unwrap();
        let system = root.path().join("system");
        let shell = system.join("System32/WindowsPowerShell/v1.0/powershell.exe");
        fs::create_dir_all(shell.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), &shell).unwrap();
        let module = root.path().join("scripts/CdrNativeProcess.psm1");
        let module_source = fs::read_to_string(&module).unwrap();
        fs::write(&module, module_source.replace(
            "$exe = $Executable",
            "$exe = $Executable\n    if ($Executable.EndsWith('cdr-runtime.exe')) { $EnvironmentOverrides['SystemRoot'] = $env:CDR_TEST_SYSTEM_ROOT }",
        )).unwrap();
        let mut command =
            fixture::command(root.path(), true, Some(std::path::Path::new("")), false);
        let windows = std::env::var("SystemRoot").unwrap();
        let cli_directory = if prior == "automatic-failure" {
            root.path().join("missing-bin")
        } else {
            selected.parent().unwrap().to_owned()
        };
        command
            .env("CDR_TEST_SYSTEM_ROOT", system)
            .env("CDR_FIXTURE_STORE_PACKAGES", "[]")
            .env(
                "CDR_CLI_EXECUTABLE_LOG",
                root.path().join("executed-image.txt"),
            )
            .env("USERPROFILE", root.path().join("home"))
            .env("LOCALAPPDATA", root.path().join("local"))
            .env(
                "PATH",
                format!(
                    "{};{windows}\\System32\\WindowsPowerShell\\v1.0;{windows}\\System32",
                    cli_directory.display()
                ),
            );
        let output = fixture::bounded(command);
        if prior == "automatic-failure" {
            assert!(!output.status.success());
            assert_eq!(
                fs::read_to_string(root.path().join(".env")).unwrap(),
                original
            );
            assert!(!root.path().join("cli-calls.jsonl").exists());
        } else {
            assert!(
                output.status.success(),
                "{prior}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                fs::read_to_string(root.path().join(".env"))
                    .unwrap()
                    .lines()
                    .any(|line| line == "CODEX_EXE=")
            );
            assert_eq!(
                fs::read_to_string(root.path().join("cli-calls.jsonl"))
                    .unwrap()
                    .lines()
                    .count(),
                4
            );
            assert!(cdr_windows_native::same_resolved_path(
                std::path::Path::new(
                    fs::read_to_string(root.path().join("executed-image.txt"))
                        .unwrap()
                        .trim()
                ),
                &selected,
            ));
        }
    }
}

#[test]
fn final_artifact_hashing_is_contained_by_the_command_deadline() {
    use std::{
        os::windows::process::CommandExt,
        time::{Duration, Instant},
    };
    let root = tempfile::tempdir().unwrap();
    let artifact = root.path().join("codex.exe");
    fs::File::create(&artifact).unwrap();
    let sparse = std::process::Command::new(
        std::path::Path::new(&std::env::var("SystemRoot").unwrap()).join("System32/fsutil.exe"),
    )
    .args(["sparse", "setflag"])
    .arg(&artifact)
    .creation_flags(0x0800_0000)
    .output()
    .unwrap();
    assert!(
        sparse.status.success(),
        "owned sparse fixture creation failed"
    );
    let bytes = 16_u64 * 1024 * 1024 * 1024;
    fs::OpenOptions::new()
        .write(true)
        .open(&artifact)
        .unwrap()
        .set_len(bytes)
        .unwrap();
    let selection = serde_json::from_value(serde_json::json!({
        "schema_version":1,"path":artifact,"source":"Environment","version":"0.166.0",
        "candidates":[],"artifacts":[{"path":artifact,"bytes":bytes,"sha256":"0".repeat(64)}],
    }))
    .unwrap();
    let start = Instant::now();
    let result = cdr_runtime::runtime_paths::selection_worker::execute_selected_using(
        std::path::Path::new(env!("CARGO_BIN_EXE_cdr-runtime")),
        selection,
        vec!["plugin".into(), "list".into()],
        root.path().join("home"),
        root.path(),
        start + Duration::from_secs(2),
    );
    assert!(start.elapsed() < Duration::from_secs(3));
    let error = result
        .err()
        .expect("hashing cannot complete before the deadline")
        .to_string();
    assert!(error.contains("probe_timeout"), "{error}");
    // The timed-out helper no longer owns the file, and no CLI was started.
    fs::write(&artifact, b"released").unwrap();
}
