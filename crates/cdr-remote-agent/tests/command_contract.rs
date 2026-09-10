use std::fs;

use cdr_remote_agent::commands::{CommandError, discover_commands, sandbox_arguments};
use cdr_remote_agent::files::ProjectFileAccess;
use cdr_remote_protocol::output::RiskTier;

#[test]
fn cmd1_discovers_only_fixed_manifest_commands_and_classifies_risk() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    fs::create_dir(&root).expect("root");
    fs::write(
        root.join("package.json"),
        r#"{"scripts":{"test":"node --test tests/a.test.mjs","download":"node --test curl","escape":"node -e bad()"}}"#,
    )
    .expect("package");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .expect("cargo");
    let commands =
        discover_commands(&ProjectFileAccess::open(&root).expect("access")).expect("discover");

    let command = |id: &str| {
        commands
            .iter()
            .find(|command| command.descriptor.command_id == id)
            .expect("command")
    };
    assert_eq!(command("npm:test").descriptor.risk_tier, RiskTier::Verify);
    assert_eq!(
        command("npm:download").descriptor.risk_tier,
        RiskTier::Network
    );
    assert_eq!(
        command("npm:escape").descriptor.risk_tier,
        RiskTier::Destructive
    );
    assert_eq!(command("cargo:test").arguments, ["cargo", "test"]);
    assert_eq!(command("cargo:clippy").arguments, ["cargo", "clippy"]);
}

#[test]
fn cmd2_oversized_or_invalid_manifest_fails_closed() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    fs::create_dir(&root).expect("root");
    fs::write(root.join("package.json"), vec![b'x'; 1_048_577]).expect("large");
    let error = discover_commands(&ProjectFileAccess::open(&root).expect("access"))
        .expect_err("oversized manifest");
    assert!(matches!(error, CommandError::File(_)));

    fs::write(root.join("package.json"), "not-json").expect("invalid");
    let error = discover_commands(&ProjectFileAccess::open(&root).expect("access"))
        .expect_err("invalid manifest");
    assert!(matches!(error, CommandError::Manifest { .. }));
}

#[test]
fn cmd3_sandbox_arguments_never_accept_arbitrary_shell_text() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    let command = sandbox_arguments(
        root,
        &["node".into(), "--test".into(), "tests/a.test.mjs".into()],
        Some("C:/tools/codex.exe"),
    )
    .expect("sandbox");
    assert_eq!(
        command,
        [
            "C:/tools/codex.exe",
            "sandbox",
            "-C",
            &root.display().to_string(),
            "-P",
            ":workspace",
            "--sandbox-state-disable-network",
            "--",
            "node",
            "--test",
            "tests/a.test.mjs",
        ]
    );
    assert!(sandbox_arguments(root, &["cmd /c escaped".into()], None).is_err());
}
