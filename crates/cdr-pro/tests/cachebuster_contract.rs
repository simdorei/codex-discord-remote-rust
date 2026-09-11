//! Native port of the independent temporary-Git cachebuster contracts.
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const MANIFEST: &str = "plugins/codex-discord-remote/.codex-plugin/plugin.json";
const PAYLOAD: &str = "plugins/codex-discord-remote/hooks/hook.mjs";
struct Fixture {
    root: tempfile::TempDir,
    base: String,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init"]);
        git(
            root.path(),
            &["config", "user.email", "cachebuster-test@example.invalid"],
        );
        git(root.path(), &["config", "user.name", "Cachebuster Test"]);
        let mut fixture = Self {
            root,
            base: String::new(),
        };
        fixture.manifest("0.1.0+codex.20260829000000", "test plugin");
        fixture.write(PAYLOAD, "old\n");
        fixture.commit();
        fixture.base = git(fixture.root.path(), &["rev-parse", "HEAD"]);
        fixture
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn manifest(&self, version: &str, description: &str) {
        self.write(
            MANIFEST,
            &json!({"name":"codex-discord-remote","version":version,"description":description})
                .to_string(),
        );
    }
    fn commit(&self) {
        git(self.root.path(), &["add", "."]);
        git(self.root.path(), &["commit", "-m", "fixture"]);
    }
    fn verify(&self, working_tree: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cdr-pro-helper"));
        command
            .args(["verify-cachebuster", "--repo-root"])
            .arg(self.root.path())
            .arg("--base-ref")
            .arg(&self.base);
        if working_tree {
            command.arg("--working-tree");
        }
        command.output().unwrap()
    }
}
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn rejected(output: Output, message: &str) {
    assert!(!output.status.success());
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains(message), "{text}");
}
fn passed(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("verification passed")
    );
}
#[test]
fn packaged_change_requires_a_strictly_newer_version() {
    for version in ["0.1.0+codex.20260829000000", "0.1.0+codex.20260828000000"] {
        let f = Fixture::new();
        f.write(PAYLOAD, "new");
        f.manifest(version, "test plugin");
        f.commit();
        rejected(f.verify(false), "without an increasing");
    }
}
#[test]
fn packaged_change_with_newer_version_passes() {
    let f = Fixture::new();
    f.write(PAYLOAD, "new");
    f.manifest("0.1.0+codex.20260829000001", "test plugin");
    f.commit();
    passed(f.verify(false));
}
#[test]
fn non_version_manifest_change_requires_bump() {
    let f = Fixture::new();
    f.manifest("0.1.0+codex.20260829000000", "changed description");
    f.commit();
    rejected(f.verify(false), "without an increasing");
}
#[test]
fn invalid_cachebuster_calendar_dates_and_versions_fail() {
    for version in [
        "0.1.0+codex.20261301000000",
        "0.1.0+codex.00000101000000",
        "0.1.0+codex.20260229000000",
        "0.1.0+codex.20260829000060",
        "01.1.0+codex.20260829000000",
    ] {
        let f = Fixture::new();
        f.manifest(version, "test plugin");
        f.commit();
        rejected(f.verify(false), "invalid");
    }
}
#[test]
fn version_only_upgrade_passes_and_downgrade_fails() {
    let f = Fixture::new();
    f.manifest("0.1.0+codex.20260829000001", "test plugin");
    f.commit();
    passed(f.verify(false));
    f.manifest("0.1.0+codex.20260828000000", "test plugin");
    f.commit();
    rejected(f.verify(false), "without increasing");
}
#[test]
fn clean_tree_and_unrelated_change_need_no_bump() {
    let f = Fixture::new();
    passed(f.verify(false));
    f.write("README.md", "unrelated");
    f.commit();
    passed(f.verify(false));
}
#[test]
fn working_tree_checks_include_untracked_packaged_files() {
    let f = Fixture::new();
    f.write("plugins/codex-discord-remote/skills/new/SKILL.md", "new");
    rejected(f.verify(true), "without an increasing");
    passed(f.verify(false));
}
#[test]
fn compiled_helper_dependency_changes_also_invalidate_the_plugin() {
    for path in [
        "crates/cdr-pro/src/hooks.rs",
        "crates/cdr-app-server/src/lib.rs",
        "crates/cdr-windows-native/src/lib.rs",
        "Cargo.lock",
    ] {
        let f = Fixture::new();
        f.write(path, "changed");
        f.commit();
        rejected(f.verify(false), "without an increasing");
    }
}
