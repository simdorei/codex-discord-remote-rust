#[path = "support/installer_native.rs"]
mod fixture;
use std::fs;
fn platforms() -> Vec<bool> {
    if cfg!(windows) {
        vec![true, false]
    } else {
        vec![false]
    }
}
fn text(out: &std::process::Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn each_required_plugin_command_failure_is_visible_and_never_install_complete() {
    for windows in platforms() {
        for step in [
            "marketplace_add",
            "plugin_add",
            "marketplace_list",
            "plugin_list",
        ] {
            let root = tempfile::tempdir().unwrap();
            let codex = fixture::seed(root.path());
            fixture::inventories(root.path(), "normal");
            let out = fixture::run(root.path(), windows, &codex, |c| {
                c.env("CDR_CLI_FAIL", step);
            });
            let result = text(&out);
            assert!(!out.status.success(), "{windows}/{step}: {result}");
            assert!(
                result.contains("INSTALL_INCOMPLETE")
                    && result.contains("codex stdout diagnostic")
                    && result.contains("unrecognized subcommand"),
                "{windows}/{step}: {result}"
            );
            assert!(
                !result.contains("Install complete.")
                    && !root.path().join(".codex_discord_runtime").exists()
            );
        }
    }
}

#[test]
fn every_legacy_invalid_inventory_case_is_refused_by_real_native_verifier() {
    for windows in platforms() {
        for case in fixture::INVALID {
            let root = tempfile::tempdir().unwrap();
            let codex = fixture::seed(root.path());
            fixture::inventories(root.path(), case);
            let out = fixture::run(root.path(), windows, &codex, |_| {});
            let result = text(&out);
            assert!(!out.status.success(), "{windows}/{case}: {result}");
            assert!(
                result.contains("INSTALL_INCOMPLETE"),
                "{windows}/{case}: {result}"
            );
            let calls =
                fs::read_to_string(root.path().join("cli-calls.jsonl")).unwrap_or_else(|e| {
                    panic!("inventory verification was not reached: {e}: {result}")
                });
            assert_eq!(calls.lines().count(), 4, "{windows}/{case}: {result}");
            assert!(
                !result.contains("Install complete.")
                    && !root.path().join(".codex_discord_runtime").exists()
            );
        }
    }
}

#[test]
fn valid_inventory_and_stderr_warning_stay_separate_and_preserve_utf8() {
    for windows in platforms() {
        for step in ["marketplace_list", "plugin_list"] {
            let outer = tempfile::tempdir().unwrap();
            let root = outer.path().join("한글 project %TEMP% ! & with spaces");
            fs::create_dir(&root).unwrap();
            let codex = fixture::seed(&root);
            fixture::inventories(&root, "normal");
            let out = fixture::run(&root, windows, &codex, |c| {
                c.env("CDR_CLI_WARN", step);
            });
            let result = text(&out);
            assert!(out.status.success(), "{windows}/{step}: {result}");
            for expected in [
                "warning:",
                "한글",
                "Verified Codex plugin inventory",
                "Install complete.",
            ] {
                assert!(result.contains(expected), "{expected}: {result}");
            }
            assert!(!result.contains("NativeCommandError"));
            assert_eq!(
                fs::read_to_string(root.join(".env")).unwrap(),
                "KEEP=한글\nCODEX_EXE=\n"
            );
            let calls: Vec<serde_json::Value> = fs::read_to_string(root.join("cli-calls.jsonl"))
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert_eq!(
                calls[0][3].as_str().unwrap().replace('\\', "/"),
                root.to_str().unwrap().replace('\\', "/")
            );
        }
    }
}

#[test]
fn dry_run_never_invokes_codex_or_claims_verified_install() {
    for windows in platforms() {
        let root = tempfile::tempdir().unwrap();
        let codex = fixture::seed(root.path());
        fixture::inventories(root.path(), "normal");
        let out = fixture::run(root.path(), windows, &codex, |c| {
            c.arg(if windows { "-DryRun" } else { "--dry-run" });
        });
        let result = text(&out);
        assert!(out.status.success(), "{result}");
        assert!(
            result.contains("Dry run complete.")
                && result.contains("Plugin inventory was not verified.")
        );
        assert!(
            !result.contains("Install complete.") && !root.path().join("cli-calls.jsonl").exists()
        );
    }
}

#[cfg(windows)]
#[test]
fn native_cmd_and_ps1_shims_preserve_literal_path_arguments() {
    for shim in ["cmd", "ps1"] {
        let outer = tempfile::tempdir().unwrap();
        let root = outer
            .path()
            .join("한글 shim project %TEMP% ! & with spaces");
        fs::create_dir(&root).unwrap();
        let exe = fixture::seed(&root);
        fixture::inventories(&root, "normal");
        let launcher = root.join(format!("codex.{shim}"));
        fs::write(
            root.join("codex.cmd"),
            "@echo off\r\n\"%CDR_TEST_CODEX_REAL%\" %*\r\nexit /b %errorlevel%\r\n",
        )
        .unwrap();
        if shim == "ps1" {
            fs::write(&launcher, "throw 'the .ps1 shim must use its .cmd sibling'").unwrap();
        }
        let out = fixture::run(&root, true, &launcher, |c| {
            c.env("CDR_TEST_CODEX_REAL", &exe)
                .env("CDR_CLI_WARN", "plugin_list");
        });
        let result = text(&out);
        assert!(out.status.success(), "{shim}: {result}");
        assert!(
            result.contains("Verified Codex plugin inventory")
                && result.contains("한글")
                && !result.contains("NativeCommandError")
        );
        let first: serde_json::Value = serde_json::from_str(
            fs::read_to_string(root.join("cli-calls.jsonl"))
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first[3], root.to_str().unwrap());
    }
}

#[cfg(windows)]
#[test]
fn native_capture_drains_both_large_error_streams_without_deadlock() {
    let root = tempfile::tempdir().unwrap();
    let codex = fixture::seed(root.path());
    fixture::inventories(root.path(), "normal");
    let out = fixture::run(root.path(), true, &codex, |c| {
        c.env("CDR_CLI_FAIL", "plugin_add")
            .env("CDR_CLI_LARGE", "1");
    });
    let result = text(&out);
    assert!(!out.status.success());
    for expected in ["INSTALL_INCOMPLETE", "STDOUT_TAIL", "STDERR_TAIL"] {
        assert!(result.contains(expected), "missing {expected}");
    }
    assert!(!result.contains("Install complete."));
}
