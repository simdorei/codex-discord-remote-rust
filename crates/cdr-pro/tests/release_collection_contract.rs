use cdr_pro::{
    release_collection::{Outcome, Runner, classify_test, collect},
    release_evidence::Status,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct FixtureRunner {
    root: PathBuf,
    version: String,
    mode: &'static str,
    calls: Vec<Vec<String>>,
    revisions: u32,
    diffs: u32,
    residents: u32,
}
impl Runner for FixtureRunner {
    fn command(&mut self, command: &[&str], root: &Path) -> Outcome {
        assert_eq!(root, self.root);
        self.calls
            .push(command.iter().map(|s| (*s).into()).collect());
        let joined = command.join(" ");
        let stdout = if joined.contains("rev-parse HEAD") {
            self.revisions += 1;
            format!(
                "revision-{}\n",
                if self.mode == "revision" {
                    self.revisions
                } else {
                    1
                }
            )
        } else if joined.contains("status --porcelain") {
            " M C:/private/project_scope-token\n".into()
        } else if joined.contains("diff --binary") {
            self.diffs += 1;
            if self.mode == "content" {
                format!("diff-{}", self.diffs)
            } else {
                String::new()
            }
        } else if joined.contains("ls-files --others") {
            String::new()
        } else if joined.contains("marketplace list --json") {
            if self.mode == "inventory" {
                "not-json".into()
            } else {
                json!({"marketplaces":[{"name":"codex-discord-remote","root":root}]}).to_string()
            }
        } else if joined.contains("plugin list --json") {
            json!({"installed":[{"pluginId":"codex-discord-remote@codex-discord-remote","installed":true,"enabled":true,"version":self.version,"source":{"path":"C:/private/plugin"}}]}).to_string()
        } else {
            assert_eq!(command[0], "cargo");
            if self.mode == "skip" {
                "test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.0s".into()
            } else {
                let count = if self.mode == "missing-suite" {
                    1
                } else {
                    command.iter().filter(|v| **v == "--test").count()
                };
                "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.0s\n".repeat(count)
            }
        };
        Outcome {
            code: 0,
            stdout,
            stderr: String::new(),
        }
    }
    fn fresh_resident(&mut self, executable: &Path, manifest: &Path) -> Result<(), String> {
        assert_eq!(executable, Path::new("codex-for-test"));
        assert!(manifest.is_file());
        self.residents += 1;
        if self.mode == "resident" {
            Err("fresh resident not healthy".into())
        } else {
            Ok(())
        }
    }
}
fn seed(root: &Path, mode: &'static str) -> FixtureRunner {
    let relative = cdr_pro::cachebuster::MANIFEST;
    fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    fs::copy(source, root.join(relative)).unwrap();
    FixtureRunner {
        root: root.into(),
        version: cdr_pro::preflight::expected_remote_plugin_version(&root.join(relative)).unwrap(),
        mode,
        calls: vec![],
        revisions: 0,
        diffs: 0,
        residents: 0,
    }
}

#[test]
fn collector_runs_native_source_inventory_and_fresh_resident_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let mut runner = seed(root.path(), "normal");
    let result = collect(root.path(), "codex-for-test", &mut runner);
    assert!(result.evidence.pre_restart_ready, "{:?}", result.problems);
    assert_eq!(result.evidence.workspace_state, "dirty");
    assert_eq!(result.evidence.repository_revision, "revision-1");
    assert_eq!(runner.calls.len(), 14);
    assert_eq!(runner.residents, 1);
    assert_eq!(runner.calls.iter().filter(|c| c[0] == "cargo").count(), 4);
    let serialized = serde_json::to_string(&result.evidence).unwrap();
    for forbidden in [
        "C:/private",
        "source.path",
        "project_scope",
        "fingerprint",
        "token",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    assert!(result.problems.is_empty());
}

#[test]
fn malformed_inventory_revision_or_same_head_content_change_fails_closed() {
    for (mode, id, status) in [
        ("inventory", "installed_plugin_inventory", Status::Malformed),
        ("revision", "repository_revision_stable", Status::Stale),
        ("content", "repository_revision_stable", Status::Stale),
        ("resident", "fresh_resident_preflight", Status::Failed),
        ("skip", "remote_mcp_capability_contract", Status::Skipped),
        (
            "missing-suite",
            "host_installer_inventory_contract",
            Status::Malformed,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let result = collect(root.path(), "codex-for-test", &mut seed(root.path(), mode));
        assert_eq!(
            result
                .evidence
                .checks
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .status,
            status
        );
        assert!(!result.evidence.pre_restart_ready);
        assert!(!result.problems.is_empty());
    }
}

#[test]
fn empty_zero_ignored_or_failed_test_runs_cannot_be_reported_as_passed() {
    for (code, stdout, status) in [
        (0, "OK (skipped=4)", Status::Skipped),
        (0, "", Status::Skipped),
        (
            0,
            "test result: ok. 0 passed; 0 failed; 0 ignored",
            Status::Skipped,
        ),
        (
            0,
            "test result: ok. 1 passed; 0 failed; 1 ignored",
            Status::Skipped,
        ),
        (
            0,
            "test result: FAILED. 1 passed; 1 failed; 0 ignored",
            Status::Failed,
        ),
        (1, "", Status::Failed),
        (
            0,
            "test result: ok. 1 passed; 0 failed; 0 ignored",
            Status::Passed,
        ),
        (
            0,
            "test result: ok. 2 passed; 0 failed; 0 ignored\ntest result: ok. 0 passed; 0 failed; 0 ignored",
            Status::Skipped,
        ),
        (
            0,
            "test result: FAILED. 2 passed; 0 failed; 0 ignored",
            Status::Failed,
        ),
    ] {
        assert_eq!(
            classify_test(&Outcome {
                code,
                stdout: stdout.into(),
                stderr: String::new()
            }),
            status
        );
    }
}

#[test]
fn unrecognized_cli_argument_does_not_start_release_checks() {
    let result =
        cdr_pro::release_collection::run_cli(["--unsupported".into()].into_iter()).unwrap_err();
    assert_eq!(result, "unknown release collector option: --unsupported");
}
