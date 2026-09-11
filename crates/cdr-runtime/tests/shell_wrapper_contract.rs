use std::{fs, path::Path, process::Command};

fn shell() -> Command {
    #[cfg(windows)]
    {
        let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .filter(|path| path.join("git.exe").is_file())
            .filter_map(|path| path.parent().map(|parent| parent.join("bin/bash.exe")))
            .find(|path| path.is_file())
            .expect("Git for Windows Bash is required for shell wrapper contracts");
        Command::new(shell)
    }
    #[cfg(not(windows))]
    {
        Command::new("sh")
    }
}

#[test]
fn shell_custom_binary_cannot_disguise_a_different_build_destination() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("install.sh"),
        fs::read_to_string(repo.join("install.sh"))
            .unwrap()
            .replace("\r\n", "\n"),
    )
    .unwrap();
    let out = shell()
        .args([
            "install.sh",
            "--dry-run",
            "--skip-env-file",
            "--skip-codex-plugin",
            "--binary-path",
            "different-build/release/cdr-runtime",
        ])
        .env_remove("CARGO_TARGET_DIR")
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("CARGO_TARGET_DIR"));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn shell_install_and_setup_are_native_nonmutating_dry_runs() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    for file in ["install.sh", "setup-discord-bot.sh", "codex-discord-bot.sh"] {
        let text = fs::read_to_string(repo.join(file)).unwrap();
        assert!(
            !text.contains("python") && !text.contains("PYTHON_EXE"),
            "{file} still requires the interpreter"
        );
        fs::write(root.path().join(file), text.replace("\r\n", "\n")).unwrap();
    }
    for file in ["install.sh", "setup-discord-bot.sh"] {
        let output = shell()
            .arg(file)
            .args((file == "install.sh").then_some("--skip-build"))
            .arg("--dry-run")
            .args([
                "--binary-path",
                &env!("CARGO_BIN_EXE_cdr-runtime").replace('\\', "/"),
            ])
            .current_dir(root.path())
            .env("PYTHON_EXE", "nonexistent-interpreter")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{file}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("Dry run"));
        assert!(!root.path().join(".env").exists());
        assert!(!root.path().join(".codex_discord_runtime").exists());
    }
}

#[test]
fn shell_launcher_disabled_marker_wins_and_invalid_mode_is_visible() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("codex-discord-bot.sh"),
        fs::read_to_string(repo.join("codex-discord-bot.sh"))
            .unwrap()
            .replace("\r\n", "\n"),
    )
    .unwrap();
    fs::write(root.path().join(".codex_discord_bot.disabled"), "disabled").unwrap();
    let output = shell()
        .arg("codex-discord-bot.sh")
        .current_dir(root.path())
        .env("CODEX_DISCORD_RUNTIME", "invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("disabled"));
    fs::remove_file(root.path().join(".codex_discord_bot.disabled")).unwrap();
    let output = shell()
        .arg("codex-discord-bot.sh")
        .current_dir(root.path())
        .env("CODEX_DISCORD_RUNTIME", "invalid")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only supports the Rust runtime"));
}

#[test]
fn shell_installer_stages_external_binary_and_preserves_existing_environment() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("install.sh"),
        fs::read_to_string(repo.join("install.sh"))
            .unwrap()
            .replace("\r\n", "\n"),
    )
    .unwrap();
    let profile = root.path().join("stored profile");
    fs::write(
        root.path().join(".env"),
        format!("# 설정 보존\nCODEX_HOME={}\nKEEP=한글\n", profile.display()),
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_cdr-runtime").replace('\\', "/");
    let run = || {
        shell()
            .arg("install.sh")
            .args([
                "--skip-build",
                "--skip-codex-plugin",
                "--binary-path",
                &binary,
                "--codex-exe",
                &binary,
            ])
            .current_dir(root.path())
            .env("CODEX_HOME", "ignored-inherited-profile")
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let installed = root.path().join("target/release/cdr-runtime");
    assert_eq!(fs::read(&installed).unwrap(), fs::read(&binary).unwrap());
    let environment = fs::read_to_string(root.path().join(".env")).unwrap();
    assert!(environment.contains("KEEP=한글"));
    let saved: Vec<_> = environment
        .lines()
        .filter_map(|line| line.strip_prefix("CODEX_HOME="))
        .map(|value| value.replace('\\', "/"))
        .collect();
    assert_eq!(saved, vec![profile.to_str().unwrap().replace('\\', "/")]);
    assert!(!environment.contains("ignored-inherited-profile"));
    fs::write(&installed, b"existing installed version").unwrap();
    let conflict = run();
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("verified deployment"));
    assert_eq!(fs::read(installed).unwrap(), b"existing installed version");
    assert_eq!(
        fs::read_to_string(root.path().join(".env")).unwrap(),
        environment
    );
}

fn run_with_native_absolute_cargo_target(script: &str) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::Builder::new()
        .prefix("cargo target ")
        .tempdir()
        .unwrap();
    let binary = target.path().join("release/cdr-runtime");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_cdr-runtime"), &binary).unwrap();
    fs::write(
        root.path().join(script),
        fs::read_to_string(repo.join(script))
            .unwrap()
            .replace("\r\n", "\n"),
    )
    .unwrap();
    let mut command = shell();
    command.arg(script);
    if script == "install.sh" {
        command.args([
            "--skip-build",
            "--skip-env-file",
            "--skip-codex-plugin",
            "--codex-exe",
        ]);
        command.arg(&binary);
    } else {
        command.arg("--dry-run");
    }
    let output = command
        .env("CARGO_TARGET_DIR", target.path())
        .env("CODEX_HOME", root.path().join("unused-profile"))
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{script}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if script == "install.sh" {
        assert_eq!(
            fs::read(root.path().join("target/release/cdr-runtime")).unwrap(),
            fs::read(&binary).unwrap()
        );
        assert_eq!(
            fs::read_to_string(root.path().join(".codex_discord_runtime")).unwrap(),
            "rust\n"
        );
    } else {
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Dry run: no token was requested"));
        assert!(text.contains("client_id=123456789012345678"));
        assert!(!root.path().join(".codex_discord_runtime").exists());
    }
    assert!(!root.path().join(".env").exists());
    assert!(!root.path().join("unused-profile").exists());
}

#[test]
fn shell_setup_resolves_native_absolute_cargo_target_with_spaces() {
    run_with_native_absolute_cargo_target("setup-discord-bot.sh");
}

#[test]
fn shell_installer_resolves_native_absolute_cargo_target_with_spaces() {
    run_with_native_absolute_cargo_target("install.sh");
}
