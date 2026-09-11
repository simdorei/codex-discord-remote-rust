#![cfg(windows)]
#[path = "support/installer_native.rs"]
mod fixture;

use std::{fs, process::Command};

#[test]
fn powershell_location_not_process_directory_owns_the_installed_profile() {
    for skip_env in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let codex = fixture::seed(root.path());
        fixture::inventories(root.path(), "normal");
        let caller = root.path().join("changed caller");
        fs::create_dir(&caller).unwrap();
        let expected = caller.join("selected profile");
        let before = fs::read(root.path().join(".env")).unwrap();
        let mut command = Command::new("powershell.exe");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"])
            .arg(
                "$ErrorActionPreference='Stop'; Set-Location -LiteralPath $env:CDR_CALLER; \
                 if ([Environment]::CurrentDirectory -eq (Get-Location).ProviderPath) { \
                   throw 'fixture did not separate process and PowerShell locations' }; \
                 & (Join-Path $env:CDR_CODEX_CLI_FIXTURE_ROOT 'install.ps1') \
                   -SkipBuild -SkipEnvFile:($env:CDR_SKIP_ENV -eq 'true') \
                   -BinaryPath (Join-Path $env:CDR_CODEX_CLI_FIXTURE_ROOT 'fixture-bin/cdr-runtime.exe') \
                   -CodexExe $env:CDR_CODEX_FIXTURE_EXE -CodexHome 'selected profile/unused/..'; \
                 if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }",
            )
            .current_dir(root.path())
            .env("CDR_CODEX_CLI_FIXTURE_ROOT", root.path())
            .env("CDR_CODEX_FIXTURE_EXE", &codex)
            .env("CDR_CALLER", &caller)
            .env("CDR_SKIP_ENV", skip_env.to_string())
            .env("CDR_CLI_PROFILE_CAPTURE", "1")
            .env_remove("CDR_CLI_FAIL")
            .env_remove("CDR_CLI_WARN")
            .env_remove("CDR_CLI_LARGE");
        let output = fixture::bounded(command);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let profiles = fs::read_to_string(root.path().join("cli-home.jsonl")).unwrap();
        assert_eq!(profiles.lines().count(), 4);
        for line in profiles.lines() {
            let actual: String = serde_json::from_str(line).unwrap();
            assert_eq!(
                actual.replace('\\', "/"),
                expected.to_str().unwrap().replace('\\', "/")
            );
        }
        let after = fs::read(root.path().join(".env")).unwrap();
        if skip_env {
            assert_eq!(after, before);
        } else {
            let saved = String::from_utf8(after).unwrap();
            let profile = saved
                .lines()
                .find_map(|line| line.strip_prefix("CODEX_HOME="))
                .unwrap();
            assert_eq!(
                profile.replace('\\', "/"),
                expected.to_str().unwrap().replace('\\', "/")
            );
            assert!(saved.contains("KEEP=한글"));
        }
        assert!(!caller.join(".env").exists());
        assert!(!root.path().join("selected profile").exists());
    }
}
