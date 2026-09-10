#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

const STAGE_PREFIX: &str = ".codex-soak-summary-publish-";
const SENTINEL: &[u8] = b"unrelated-stage-sentinel";

fn final_summaries(fixture: &wrapper::Fixture) -> Vec<PathBuf> {
    fs::read_dir(&fixture.evidence)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.ends_with(".summary.json") && !name.ends_with(".harness.summary.json")
        })
        .collect()
}

fn stage_files(fixture: &wrapper::Fixture) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(&fixture.evidence)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.starts_with(STAGE_PREFIX) && name.ends_with(".tmp")
        })
        .collect();
    paths.sort();
    paths
}

fn assert_complete_v3(path: &Path) {
    let bytes = fs::read(path).unwrap();
    assert!(!bytes.starts_with(&[0xef, 0xbb, 0xbf]), "UTF-8 BOM found");
    std::str::from_utf8(&bytes).expect("summary must be strict UTF-8");
    let summary: Value = serde_json::from_slice(&bytes).expect("parseable summary JSON");
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3");
    for field in [
        "mode",
        "evidence_id",
        "operational_status",
        "duration_seconds",
        "seed",
        "harness_pid",
        "harness_started_at_utc",
        "child_exit_code",
        "harness_provenance",
        "harness_summary",
        "harness_event_count",
        "memory",
        "disabled_marker",
        "outputs",
        "source_provenance",
        "final_eligibility",
    ] {
        assert!(summary.get(field).is_some(), "missing v3 field {field}");
    }
}

fn ps_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

#[derive(Debug, Deserialize)]
struct ModuleAst {
    errors: Vec<String>,
    publish: Vec<String>,
    function_names: Vec<String>,
    command_names: Vec<String>,
}

fn module_ast() -> ModuleAst {
    let path = wrapper::repo_root().join("scripts/CodexDiscordSoak.WrapperEvidence.psm1");
    let inspect = format!(
        r"$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile({},[ref]$tokens,[ref]$errors)
$functions=@($ast.FindAll({{param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst]}},$true))
$publish=@($functions | Where-Object Name -CEQ 'Publish-CodexSoakSummaryV3')
$commands=@($ast.FindAll({{param($node) $node -is [Management.Automation.Language.CommandAst]}},$true))
[ordered]@{{
  errors=@($errors | ForEach-Object Message)
  publish=@($publish | ForEach-Object {{ $_.Extent.Text }})
  function_names=@($functions | ForEach-Object Name)
  command_names=@($commands | ForEach-Object {{ [string]$_.GetCommandName() }})
}} | ConvertTo-Json -Depth 5 -Compress
",
        ps_literal(&path)
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &inspect])
        .output()
        .unwrap();
    assert!(output.status.success(), "AST inspection failed: {output:?}");
    let parsed: ModuleAst = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        parsed.errors.is_empty(),
        "parse errors: {:?}",
        parsed.errors
    );
    parsed
}

#[test]
fn w3_pub_01_absent_destination_publishes_one_complete_v3_without_owned_residue() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    fs::create_dir_all(&fixture.evidence).unwrap();
    let sentinel = fixture
        .evidence
        .join(format!("{STAGE_PREFIX}unrelated.tmp"));
    fs::write(&sentinel, SENTINEL).unwrap();
    let baseline_stages = stage_files(&fixture);

    let output = wrapper::wait_for_output(
        wrapper::skip_build_command(&fixture, 1).spawn().unwrap(),
        Duration::from_secs(15),
    );

    assert!(output.status.success(), "wrapper failed: {output:?}");
    let summaries = final_summaries(&fixture);
    assert_eq!(summaries.len(), 1);
    assert_complete_v3(&summaries[0]);
    assert_eq!(stage_files(&fixture), baseline_stages);
    assert_eq!(fs::read(sentinel).unwrap(), SENTINEL);
}

#[test]
fn w3_pub_05_publisher_scope_is_create_only_atomic_and_has_no_replace_fallback() {
    let root = wrapper::repo_root();
    let main = fs::read_to_string(root.join("codex-discord-rust-soak.ps1")).unwrap();
    assert_eq!(main.matches("Publish-CodexSoakSummaryV3").count(), 1);
    assert!(!main.contains("WriteAllText($resultPath"));

    let ast = module_ast();
    assert_eq!(ast.publish.len(), 1, "publisher definition count");
    assert!(
        !ast.function_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Initialize-CodexSoakAtomicSummaryPublisher"))
    );
    assert!(
        !ast.command_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Add-Type"))
    );
    let publish = &ast.publish[0];
    for forbidden in [
        "WriteAllText",
        "WriteAllBytes",
        "WriteAllLines",
        "StreamWriter",
        "ReplaceFileW",
        "ReplaceWithoutBackup",
        "[IO.File]::Replace",
        "[IO.File]::Exists",
        "Test-Path",
        "Add-Type",
        "Move-Item",
        "Copy-Item",
    ] {
        assert!(
            !publish.contains(forbidden),
            "forbidden publisher primitive {forbidden}"
        );
    }
    for required in [
        "[Text.UTF8Encoding]::new($false, $true)",
        "[IO.FileMode]::CreateNew",
        "[IO.FileAccess]::Write",
        "[IO.FileShare]::None",
        "$stream.Write($bytes, 0, $bytes.Length)",
        "$stream.Flush($true)",
        "$stream.Dispose(); $stream = $null",
        "[IO.File]::Move($stagePath, $destinationFullPath)",
        "[IO.File]::Delete($stagePath)",
    ] {
        assert!(
            publish.contains(required),
            "missing publisher primitive {required}"
        );
    }
    assert_eq!(publish.matches("[IO.FileStream]::new(").count(), 1);
    assert_eq!(publish.matches("[IO.File]::Move(").count(), 1);
    assert_eq!(publish.matches("[IO.File]::Delete(").count(), 1);
    assert_eq!(publish.matches("$stream.Dispose()").count(), 2);

    let at = |needle: &str| {
        publish
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle}"))
    };
    assert!(at("ConvertTo-Json") < at("$bytes ="));
    assert!(at("$bytes =") < at("[IO.Path]::GetFullPath"));
    assert!(at("[IO.Path]::GetFullPath") < at("$stageName ="));
    assert!(at("$stageName =") < at("$stream = [IO.FileStream]::new"));
    assert!(at("$stream = [IO.FileStream]::new") < at("$stream.Write("));
    assert!(at("$stream.Write(") < at("$stream.Flush($true)"));
    assert!(at("$stream.Flush($true)") < at("$stream.Dispose(); $stream = $null"));
    assert!(at("$stream.Dispose(); $stream = $null") < at("[IO.File]::Move("));
    let cleanup_catch = at("$original = $_.Exception");
    let cleanup_dispose = publish[cleanup_catch..]
        .find("$stream.Dispose()")
        .map(|offset| cleanup_catch + offset)
        .expect("missing catch-path stream disposal");
    assert!(at("[IO.File]::Move(") < cleanup_catch);
    assert!(cleanup_catch < cleanup_dispose);
    assert!(cleanup_dispose < at("[IO.File]::Delete($stagePath)"));
}
