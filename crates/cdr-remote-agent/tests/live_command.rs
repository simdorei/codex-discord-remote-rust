use std::fs;
use std::time::Duration;

use cdr_remote_agent::commands::run_command;
use cdr_remote_agent::files::ProjectFileAccess;
use cdr_remote_protocol::output::CoreOutput;

#[tokio::test]
#[ignore = "requires the current Codex CLI and Node.js"]
async fn current_codex_sandbox_runs_a_discovered_node_verification() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    fs::create_dir(root.join("tests")).expect("tests");
    fs::write(
        root.join("tests/verified.test.mjs"),
        "import test from 'node:test'; test('verified', () => console.log('verified'));",
    )
    .expect("test file");
    fs::write(
        root.join("package.json"),
        r#"{"scripts":{"test":"node --test --test-isolation=none tests/verified.test.mjs"}}"#,
    )
    .expect("package");
    let output = run_command(
        &ProjectFileAccess::open(root).expect("access"),
        "npm:test",
        Duration::from_secs(30),
        None,
    )
    .await
    .expect("sandboxed command");
    match output {
        CoreOutput::CommandRun {
            exit_code,
            stdout,
            truncated,
            ..
        } => {
            assert_eq!(exit_code, 0);
            assert!(stdout.contains("verified"));
            assert!(!truncated);
        }
        other => panic!("unexpected output: {other:?}"),
    }
}
