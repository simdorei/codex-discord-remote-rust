#[path = "support/installer_native.rs"]
mod fixture;

use std::{fs, path::Path};

fn platforms() -> Vec<bool> {
    if cfg!(windows) {
        vec![true, false]
    } else {
        vec![false]
    }
}

fn assert_profiles(root: &Path, expected: &Path) {
    let contents = fs::read_to_string(root.join("cli-home.jsonl")).unwrap();
    let profiles: Vec<Option<String>> = contents
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(profiles.len(), 4);
    for actual in profiles {
        assert_eq!(
            actual.unwrap().replace('\\', "/"),
            expected.to_str().unwrap().replace('\\', "/")
        );
    }
}

#[test]
fn explicit_profile_reaches_all_plugin_commands_without_saving_environment() {
    for windows in platforms() {
        let root = tempfile::tempdir().unwrap();
        let codex = fixture::seed(root.path());
        fixture::inventories(root.path(), "normal");
        let profile = root.path().join("검증 profile");
        let before = fs::read(root.path().join(".env")).unwrap();
        let output = fixture::run(root.path(), windows, &codex, |command| {
            command
                .arg(if windows {
                    "-CodexHome"
                } else {
                    "--codex-home"
                })
                .arg(&profile)
                .env("CODEX_HOME", root.path().join("unrelated profile"))
                .env("CDR_CLI_PROFILE_CAPTURE", "1");
        });
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_profiles(root.path(), &profile);
        assert_eq!(fs::read(root.path().join(".env")).unwrap(), before);
    }
}

#[test]
fn saved_profile_wins_over_inherited_profile_without_explicit_override() {
    for windows in platforms() {
        let root = tempfile::tempdir().unwrap();
        let codex = fixture::seed(root.path());
        fixture::inventories(root.path(), "normal");
        let profile = root.path().join("stored profile");
        let before = format!("KEEP=한글\nCODEX_HOME={}\n", profile.display());
        fs::write(root.path().join(".env"), &before).unwrap();
        let output = fixture::run(root.path(), windows, &codex, |command| {
            command
                .env("CODEX_HOME", root.path().join("unrelated profile"))
                .env("CDR_CLI_PROFILE_CAPTURE", "1");
        });
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_profiles(root.path(), &profile);
        assert_eq!(
            fs::read_to_string(root.path().join(".env")).unwrap(),
            before
        );
    }
}

#[test]
fn invalid_runtime_profile_is_rejected_without_registering_elsewhere() {
    for windows in platforms() {
        let root = tempfile::tempdir().unwrap();
        let codex = fixture::seed(root.path());
        fixture::inventories(root.path(), "normal");
        let output = fixture::run(root.path(), windows, &codex, |command| {
            command
                .arg(if windows {
                    "-CodexHome"
                } else {
                    "--codex-home"
                })
                .arg(root.path().join(".sandbox-bin"));
        });
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("CODEX_HOME"));
        assert!(!root.path().join("cli-calls.jsonl").exists());
    }
}

#[test]
fn spaced_quoted_saved_profile_reaches_plugin_commands_unchanged() {
    for windows in platforms() {
        for quote in ['\'', '"'] {
            let root = tempfile::tempdir().unwrap();
            let codex = fixture::seed(root.path());
            fixture::inventories(root.path(), "normal");
            let profile = root.path().join("stored profile");
            let before = format!(
                "KEEP=한글\n  CODEX_HOME = {quote}{}{quote}  \n",
                profile.display()
            );
            fs::write(root.path().join(".env"), &before).unwrap();
            let output = fixture::run(root.path(), windows, &codex, |command| {
                command
                    .env("CODEX_HOME", root.path().join("unrelated profile"))
                    .env("CDR_CLI_PROFILE_CAPTURE", "1");
            });
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_profiles(root.path(), &profile);
            assert_eq!(
                fs::read_to_string(root.path().join(".env")).unwrap(),
                before
            );
        }
    }
}

#[test]
fn codex_runtime_bin_is_not_a_profile_with_either_path_separator() {
    let mut suffixes = vec![
        ".sandbox-bin/version-one/",
        "appdata/local/openai/codex/bin",
    ];
    if cfg!(windows) {
        suffixes.extend([
            "AppData/Local/OpenAI/Codex/bin",
            "APPDATA/LOCAL/OPENAI/CODEX/BIN/version-one/",
        ]);
    }
    for windows in platforms() {
        for suffix in &suffixes {
            let root = tempfile::tempdir().unwrap();
            let codex = fixture::seed(root.path());
            fixture::inventories(root.path(), "normal");
            let profile = root
                .path()
                .join(suffix)
                .to_string_lossy()
                .replace('\\', "/");
            let output = fixture::run(root.path(), windows, &codex, |command| {
                command
                    .arg(if windows {
                        "-CodexHome"
                    } else {
                        "--codex-home"
                    })
                    .arg(&profile);
            });
            assert!(!output.status.success(), "runtime bin accepted: {profile}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("CODEX_HOME"));
            assert!(!root.path().join("cli-calls.jsonl").exists());
        }
    }
}

#[test]
fn relative_profile_is_resolved_once_from_the_callers_directory() {
    for windows in platforms() {
        for skip_env in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let codex = fixture::seed(root.path());
            fixture::inventories(root.path(), "normal");
            let caller = root.path().join("unrelated caller");
            fs::create_dir(&caller).unwrap();
            let profile = caller.join("selected profile");
            let mut command = fixture::command(root.path(), windows, Some(&codex), skip_env);
            command
                .current_dir(&caller)
                .arg(if windows {
                    "-CodexHome"
                } else {
                    "--codex-home"
                })
                .arg("selected profile/unused/..")
                .env("CDR_CLI_PROFILE_CAPTURE", "1");
            let output = fixture::bounded(command);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_profiles(root.path(), &profile);
            if !skip_env {
                let saved = fs::read_to_string(root.path().join(".env")).unwrap();
                let line = saved
                    .lines()
                    .find(|line| line.starts_with("CODEX_HOME="))
                    .unwrap();
                assert_eq!(
                    line[11..].replace('\\', "/"),
                    profile.to_str().unwrap().replace('\\', "/")
                );
            }
            assert!(!caller.join(".env").exists());
        }
    }
}
