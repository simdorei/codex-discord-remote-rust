//! Inspect actual deliverable files, not the presence/absence of a Python command on PATH.
use std::{fs, path::Path, process::Command};

#[test]
fn deliverable_contains_no_python_source_or_python_dependency_installation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("git")
        .current_dir(&root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "file inventory failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = String::from_utf8(output.stdout).unwrap();
    let python: Vec<_> = files
        .split('\0')
        .filter(|p| {
            Path::new(p)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
                && root.join(p).is_file()
        })
        .collect();
    assert!(
        python.is_empty(),
        "{} Python sources remain; first: {:?}",
        python.len(),
        python.first()
    );
    for obsolete in [
        "requirements.txt",
        "requirements.in",
        "runtime-release.json",
        "remote_mcp_server/pyproject.toml",
        "remote_mcp_server/uv.lock",
        "codex-discord-python-runtime.ps1",
        "codex-discord-watchdog-identity-runtime.ps1",
        "codex-discord-watchdog-restart-runtime.ps1",
        "codex-discord-watchdog-runtime.ps1",
        "codex-discord-watchdog-stop-runtime.ps1",
        "codex-discord-watchdog-heartbeat-runtime.ps1",
    ] {
        assert!(
            !root.join(obsolete).exists(),
            "obsolete Python installation/launcher file remains: {obsolete}"
        );
    }
    for directory in [
        ".python-portable",
        ".python-portable.stage",
        ".python-portable.previous",
        ".venv",
        "__pycache__",
    ] {
        assert!(
            !root.join(directory).exists(),
            "a project-local interpreter/cache remains: {directory}"
        );
    }
}

#[test]
fn runtime_helpers_and_ci_do_not_reintroduce_an_interpreter_launcher() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in [
        "install.ps1",
        "install.sh",
        "setup-discord-bot.ps1",
        "setup-discord-bot.sh",
        "codex-discord-bot.cmd",
        "codex-discord-bot.sh",
        "plugins/codex-discord-remote/hooks/browser-evidence.json",
        "plugins/codex-discord-remote/hooks/pro-connector-evidence.json",
        ".github/workflows/windows-contract.yml",
        ".github/workflows/macos-smoke.yml",
    ] {
        let text = fs::read_to_string(root.join(path)).unwrap().to_lowercase();
        for forbidden in [
            "python.exe",
            "pythonw.exe",
            "python3 ",
            "python -",
            "py -3",
            "setup-python",
            "pip install",
            "uv sync",
            ".python-portable",
        ] {
            assert!(!text.contains(forbidden), "{path} reintroduced {forbidden}");
        }
    }
}
