//! Native replacement for the pre-restart release collector, with injectable boundaries.
use crate::release_evidence::{Check, Evidence, Status};
use std::path::Path;
mod classification;
mod command;
mod workspace;
pub use classification::classify_test;
pub use command::{RealRunner, run_cli};

#[derive(Clone, Debug)]
pub struct Outcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
pub trait Runner {
    fn command(&mut self, command: &[&str], root: &Path) -> Outcome;
    fn fresh_resident(&mut self, executable: &Path, manifest: &Path) -> Result<(), String>;
}
pub struct Collection {
    pub evidence: Evidence,
    pub problems: Vec<String>,
}

pub fn collect(root: &Path, codex: &str, runner: &mut impl Runner) -> Collection {
    let before = workspace::snapshot(root, runner);
    let mut checks = Vec::new();
    let mut problems = Vec::new();
    let manifest = root.join(crate::cachebuster::MANIFEST);
    let version = crate::preflight::expected_remote_plugin_version(&manifest);
    checks.push(Check::new(
        "plugin_manifest",
        "source",
        if version.is_ok() {
            Status::Passed
        } else {
            Status::Malformed
        },
    ));
    let version = version.unwrap_or_else(|error| {
        problems.push(format!("plugin_manifest: {error}"));
        "unavailable".into()
    });
    for (id, package, tests) in source_checks() {
        let expected_suites = tests.len();
        let mut command = vec![
            "cargo",
            "test",
            "--locked",
            "--offline",
            "-j",
            "2",
            "-p",
            package,
        ];
        for test in tests {
            command.extend(["--test", test]);
        }
        command.extend(["--", "--test-threads=2"]);
        let outcome = runner.command(&command, root);
        let status = classification::classify_required_targets(&outcome, expected_suites);
        if status != Status::Passed {
            problems.push(format!(
                "{id}: {status:?}; exit={}; {}",
                outcome.code, outcome.stderr
            ));
        }
        checks.push(Check::new(id, "source", status));
    }
    let inventory = installed_inventory(root, codex, &manifest, runner);
    if let Err((_, detail)) = &inventory {
        problems.push(format!("installed_plugin_inventory: {detail}"));
    }
    checks.push(Check::new(
        "installed_plugin_inventory",
        "installed",
        inventory.err().map_or(Status::Passed, |(status, _)| status),
    ));
    let resident = runner.fresh_resident(Path::new(codex), &manifest);
    if let Err(detail) = &resident {
        problems.push(format!("fresh_resident_preflight: {detail}"));
    }
    checks.push(Check::new(
        "fresh_resident_preflight",
        "runtime",
        if resident.is_ok() {
            Status::Passed
        } else {
            Status::Failed
        },
    ));
    let after = workspace::snapshot(root, runner);
    let stable = before.digest.is_some()
        && before.digest == after.digest
        && !before.revision.is_empty()
        && before.revision == after.revision;
    if !stable {
        problems.push(
            "repository_revision_stable: Git revision/content could not be verified unchanged"
                .into(),
        );
    }
    checks.push(Check::new(
        "repository_revision_stable",
        "source",
        if stable {
            Status::Passed
        } else {
            Status::Stale
        },
    ));
    Collection {
        evidence: Evidence::new(
            before.revision,
            before.state,
            std::env::consts::OS.into(),
            version,
            checks,
        ),
        problems,
    }
}

fn source_checks() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    vec![
        (
            "remote_mcp_capability_contract",
            "cdr-mcp-server",
            vec!["capability_contract"],
        ),
        (
            "host_installer_inventory_contract",
            "cdr-runtime",
            vec![
                "install_plugin_native_contract",
                "install_codex_persistence_contract",
            ],
        ),
        (
            "browser_evidence_contract",
            "cdr-pro",
            vec![
                "plugin_packaging_contract",
                "hook_contract",
                "hook_retry_ordering_contract",
                "browser_script_contract",
            ],
        ),
        (
            "pro_runtime_contract",
            "cdr-pro",
            vec![
                "fingerprint_contract",
                "preflight_contract",
                "prompt_contract",
                "gate_contract",
            ],
        ),
    ]
}

fn installed_inventory(
    root: &Path,
    codex: &str,
    manifest: &Path,
    runner: &mut impl Runner,
) -> Result<(), (Status, String)> {
    let market = runner.command(&[codex, "plugin", "marketplace", "list", "--json"], root);
    let plugin = runner.command(&[codex, "plugin", "list", "--json"], root);
    for outcome in [&market, &plugin] {
        if outcome.code != 0 {
            return Err((
                Status::Failed,
                format!("query exit={}: {}", outcome.code, outcome.stderr),
            ));
        }
        if !serde_json::from_str::<serde_json::Value>(&outcome.stdout).is_ok_and(|v| v.is_object())
        {
            return Err((
                Status::Malformed,
                "query did not return a JSON object".into(),
            ));
        }
    }
    let temp = tempfile::tempdir().map_err(|e| (Status::Failed, e.to_string()))?;
    let market_path = temp.path().join("market.json");
    let plugin_path = temp.path().join("plugin.json");
    std::fs::write(&market_path, market.stdout).map_err(|e| (Status::Failed, e.to_string()))?;
    std::fs::write(&plugin_path, plugin.stdout).map_err(|e| (Status::Failed, e.to_string()))?;
    crate::installation::verify_inventory(
        &market_path,
        &plugin_path,
        manifest,
        root,
        "codex-discord-remote",
        crate::preflight::REMOTE_PLUGIN_ID,
    )
    .map(|_| ())
    .map_err(|e| (Status::Failed, e))
}
