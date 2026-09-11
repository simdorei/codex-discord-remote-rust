use std::{
    fs,
    path::{Path, PathBuf},
};
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn workflow(name: &str) -> String {
    fs::read_to_string(repo().join(format!(".github/workflows/{name}.yml"))).unwrap()
}
#[test]
fn both_ci_platforms_build_test_lint_without_interpreter_setup() {
    for name in ["windows-contract", "macos-smoke"] {
        let text = workflow(name);
        for forbidden in [
            "setup-python",
            "python -m",
            "Scripts/python.exe",
            "PY_PYTHON3",
            "verify_plugin_cachebuster.py",
        ] {
            assert!(!text.contains(forbidden), "{name}: {forbidden}");
        }
        for command in [
            "rustup show active-toolchain",
            "cargo build --workspace --locked",
            "cargo test --workspace --locked --all-targets",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
        ] {
            assert!(text.contains(command), "{name}: {command}");
        }
        assert!(text.contains("actions/setup-node@") && text.contains("fetch-depth: 0"));
    }
}
#[test]
fn windows_preserves_each_native_exit_and_bounded_formatting() {
    let text = workflow("windows-contract");
    for command in [
        "rustup show active-toolchain",
        "cargo build --workspace --locked",
        "cargo test --workspace --locked --all-targets -- --test-threads=2",
        "cargo clippy --workspace --all-targets --locked -- -D warnings",
    ] {
        assert!(
            text.contains(&format!(
                "{command}\n          if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}"
            )),
            "{command}"
        );
    }
    assert!(
        text.contains("./scripts/Test-RustFormatting.ps1") && !text.contains("cargo fmt --all")
    );
    let formatting = fs::read_to_string(repo().join("scripts/Test-RustFormatting.ps1")).unwrap();
    assert!(formatting.contains("$LASTEXITCODE -ne 0"));
}
#[test]
fn native_pro_cache_and_installer_contracts_are_mandatory_on_both_platforms() {
    for name in ["windows-contract", "macos-smoke"] {
        let text = workflow(name);
        for item in [
            "PLUGIN_VERSION_BASE_REF",
            "github.event.pull_request.base.sha",
            "github.event.before",
            "verify-cachebuster",
            "--repo-root .",
            "cargo test --locked -p cdr-pro --all-targets",
            "--test shell_wrapper_contract",
            "--test admin_cli_contract",
        ] {
            assert!(text.contains(item), "{name}: {item}");
        }
        assert!(text.contains(if name == "windows-contract" {
            "install.ps1 -DryRun"
        } else {
            "install.sh --dry-run"
        }));
    }
}
#[test]
fn native_pro_suite_contains_all_release_critical_replacements() {
    for file in [
        "browser_script_contract",
        "hook_contract",
        "hook_retry_ordering_contract",
        "evidence_contract",
        "gate_contract",
        "plugin_packaging_contract",
        "conversation_contract",
        "cachebuster_contract",
    ] {
        assert!(
            repo()
                .join(format!("crates/cdr-pro/tests/{file}.rs"))
                .is_file(),
            "{file}"
        );
    }
}
#[test]
fn smoke_entry_has_no_legacy_command_and_does_not_silently_skip_shell() {
    let text = fs::read_to_string(repo().join("plugins/codex-discord-remote/scripts/qa-smoke.ps1"))
        .unwrap();
    for forbidden in ["py -3", "py_compile", "-m unittest"] {
        assert!(!text.contains(forbidden));
    }
    assert!(text.contains("Test-NativeWorkspace.ps1"));
    let source = fs::read_to_string(repo().join("scripts/Test-NativeWorkspace.ps1")).unwrap();
    assert!(
        source.contains("Git Bash is required")
            && source.contains("cargo test --workspace --locked --all-targets")
    );
    assert!(source.contains("partial_checks_only") && source.contains("-SkipBuild"));
}
