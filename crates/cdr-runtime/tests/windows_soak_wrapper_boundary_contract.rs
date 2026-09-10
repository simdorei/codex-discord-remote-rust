#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[rustfmt::skip]
struct AstEvent { kind: String, name: String, text: String, start: usize }
#[derive(Deserialize)]
#[rustfmt::skip]
struct AstEnvelope { errors: Vec<String>, events: Vec<AstEvent> }

fn ps_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

#[rustfmt::skip]
fn wrapper_ast() -> (String, Vec<AstEvent>) {
    let path = wrapper::repo_root().join("codex-discord-rust-soak.ps1");
    let bytes = fs::read(&path).unwrap();
    assert!(!bytes.starts_with(&[0xef, 0xbb, 0xbf]));
    let source = String::from_utf8(bytes).expect("wrapper must be strict UTF-8");
    let inspect = format!(r"$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile({},[ref]$tokens,[ref]$errors)
$nodes=@($ast.FindAll({{
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -or
    $node -is [Management.Automation.Language.InvokeMemberExpressionAst]
}},$true))
$events=@($nodes | ForEach-Object {{
    if ($_ -is [Management.Automation.Language.CommandAst]) {{
        $kind='command';$name=[string]$_.GetCommandName()
    }} else {{ $kind='member';$name=[string]$_.Member.Value }}
    [ordered]@{{kind=$kind;name=$name;text=$_.Extent.Text;start=$_.Extent.StartOffset}}
}})
[ordered]@{{errors=@($errors | ForEach-Object Message);events=$events}} |
    ConvertTo-Json -Depth 5 -Compress
", ps_literal(&path));
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &inspect]).output().unwrap();
    assert!(output.status.success(), "AST inspection failed: {output:?}");
    let mut parsed: AstEnvelope = serde_json::from_slice(&output.stdout).unwrap();
    assert!(parsed.errors.is_empty(), "parse errors: {:?}", parsed.errors);
    parsed.events.sort_by_key(|event| event.start);
    (source, parsed.events)
}

fn named<'a>(events: &'a [AstEvent], kind: &str, name: &str) -> Vec<&'a AstEvent> {
    events
        .iter()
        .filter(|event| event.kind == kind && event.name.eq_ignore_ascii_case(name))
        .collect()
}

fn only<'a>(events: &'a [AstEvent], kind: &str, name: &str) -> &'a AstEvent {
    let found = named(events, kind, name);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one {kind} {name}: {found:?}"
    );
    found[0]
}

fn run(mut command: Command) -> Output {
    let child = command.spawn().unwrap();
    wrapper::wait_for_output(child, Duration::from_secs(20))
}

#[rustfmt::skip]
fn assert_failed(output: &Output) {
    assert!(!output.status.success(), "wrapper unexpectedly passed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

fn assert_before_child(summary: &Value) {
    for field in ["harness_pid", "harness_started_at_utc", "child_exit_code"] {
        assert!(summary[field].is_null(), "{field} must remain null");
    }
    assert!(summary["harness_provenance"]["pid"].is_null());
}

fn assert_soak_unreached(summary: &Value) {
    let provenance = &summary["source_provenance"];
    assert!(provenance["soak_after"].is_null());
    assert!(provenance["soak_fingerprints_equal"].is_null());
}

fn append_module(fixture: &wrapper::Fixture, name: &str, code: &str) {
    let path = fixture.root.join("scripts").join(name);
    let mut source = fs::read_to_string(&path).unwrap();
    source.push_str(code);
    fs::write(path, source).unwrap();
}

#[test]
#[rustfmt::skip]
fn boundary_01_soak_snapshot_cleanup_and_eligibility_precede_atomic_publication() {
    let (source, events) = wrapper_ast();
    let stop = only(&events, "command", "Stop-CodexSoakOwnedProcess");
    let complete = only(&events, "command", "Complete-CodexSoakHarnessProvenance");
    let gets = named(&events, "command", "Get-CodexSoakSourceFingerprint");
    let compares = named(&events, "command", "Test-CodexSoakSourceFingerprintEqual");
    let soak_after = gets.last().expect("terminal soak_after snapshot");
    let compare = compares.last().expect("terminal soak fingerprint comparison");
    let soak_stage = source.find("$stage = 'source_fingerprint_soak_after'").unwrap();
    let compare_stage = source.find("$stage = 'source_fingerprint_soak_compare'").unwrap();
    assert!(stop.start < complete.start);
    assert!(complete.start < soak_stage && soak_stage < soak_after.start);
    assert!(soak_after.start < compare_stage && compare_stage < compare.start);

    let close_guard = only(&events, "command", "Close-CodexSoakHarnessGuard");
    let close_owner = only(&events, "command", "Close-CodexSoakProcessOwner");
    let drains = named(&events, "command", "Write-CodexSoakCapturedOutput");
    assert_eq!(drains.len(), 2, "stdout and stderr must both drain");
    let disposals = named(&events, "member", "Dispose");
    let final_dispose = |needle: &str| disposals.iter().rev().copied()
        .find(|event| event.text.to_ascii_lowercase().contains(needle))
        .unwrap_or_else(|| panic!("missing final {needle}"));
    let memory_dispose = final_dispose("$memorywriter.dispose");
    let child_dispose = final_dispose("$child.dispose");
    let marker_dispose = final_dispose("$markerguard.dispose");
    let marker_cleanup = named(&events, "command", "Assert-CodexSoakDisabledMarker")
        .into_iter().last().expect("terminal marker cleanup assertion");
    let cleanup_runtime = named(&events, "command", "Assert-CodexSoakRuntimeStopped")
        .into_iter().last().expect("terminal runtime cleanup assertion");
    let cleanup = [close_guard, close_owner, memory_dispose, drains[0], drains[1],
        child_dispose, marker_cleanup, cleanup_runtime, marker_dispose];
    assert!(cleanup.iter().all(|event| compare.start < event.start));
    let cleanup_end = cleanup.iter().map(|event| event.start).max().unwrap();

    let eligibility = only(&events, "command", "Get-CodexSoakFinalEligibility");
    let summary = only(&events, "command", "New-CodexSoakSummaryV3");
    let publisher = only(&events, "command", "Publish-CodexSoakSummaryV3");
    assert!(cleanup_end < eligibility.start);
    assert!(eligibility.start < summary.start && summary.start < publisher.start);
    let direct_members: Vec<_> = events.iter()
        .filter(|event| event.kind == "member" && event.text.contains("$resultPath"))
        .collect();
    assert!(direct_members.is_empty(), "direct final-path member access: {direct_members:?}");
    let final_path_commands: Vec<_> = events.iter()
        .filter(|event| event.kind == "command" && event.text.contains("$resultPath"))
        .collect();
    assert!(final_path_commands.iter().all(|event|
        event.name.eq_ignore_ascii_case("Publish-CodexSoakSummaryV3") ||
        event.name.eq_ignore_ascii_case("Write-Output")),
        "unexpected final-path command: {final_path_commands:?}");
    assert!(named(&events, "command", "New-CodexSoakTerminalException").into_iter()
        .all(|event| publisher.start < event.start));
    assert!(named(&events, "command", "Write-Output").into_iter()
        .all(|event| publisher.start < event.start));
}

#[test]
#[rustfmt::skip]
fn boundary_02_build_failure_does_not_fabricate_a_soak_boundary() {
    let temp = tempfile::tempdir().unwrap(); let fixture = wrapper::prepare(temp.path());
    let cargo = wrapper::fake_cargo(&fixture, false, 31);
    let output = run(wrapper::build_command(&fixture, &cargo, 1)); assert_failed(&output);
    let summary = wrapper::summary(&fixture); let primary = &summary["source_provenance"]["primary_failure"];
    assert_eq!((primary["stage"].as_str(), primary["code"].as_str()),
        (Some("build"), Some("build_failed")));
    assert_before_child(&summary); assert_soak_unreached(&summary); wrapper::assert_marker(&fixture);
}

#[test]
#[rustfmt::skip]
fn boundary_03_build_drift_does_not_fabricate_a_soak_boundary() {
    let temp = tempfile::tempdir().unwrap(); let fixture = wrapper::prepare(temp.path());
    let cargo = wrapper::fake_cargo(&fixture, true, 0);
    let output = run(wrapper::build_command(&fixture, &cargo, 1)); assert_failed(&output);
    let summary = wrapper::summary(&fixture); let primary = &summary["source_provenance"]["primary_failure"];
    assert_eq!((primary["stage"].as_str(), primary["code"].as_str()),
        (Some("source_fingerprint_build_compare"), Some("source_changed_during_build")));
    assert_before_child(&summary); assert_soak_unreached(&summary); wrapper::assert_marker(&fixture);
}

#[test]
#[rustfmt::skip]
fn boundary_04_throwing_build_comparator_is_not_false_or_source_drift() {
    let temp = tempfile::tempdir().unwrap(); let fixture = wrapper::prepare(temp.path());
    append_module(&fixture, "CodexDiscordSoak.Evidence.psm1", r"
function Test-CodexSoakSourceFingerprintEqual {
    param([object]$Expected,[object]$Actual)
    $failure=[InvalidOperationException]::new('forced comparator invalid record')
    $failure.Data['CodexSoakFailureCode']='fingerprint_record_invalid'
    throw $failure
}
Export-ModuleMember -Function @('Test-CodexSoakSourceFingerprintEqual','Get-CodexSoakFinalEligibility')
");
    let cargo = wrapper::fake_cargo(&fixture, false, 0);
    let output = run(wrapper::build_command(&fixture, &cargo, 1)); assert_failed(&output);
    let summary = wrapper::summary(&fixture); let provenance = &summary["source_provenance"];
    assert_eq!(provenance["primary_failure"], json!({
        "stage": "source_fingerprint_build_compare", "code": "fingerprint_record_invalid",
        "message": "forced comparator invalid record"
    }));
    assert_eq!(provenance["secondary_failures"], json!([]));
    assert!(provenance["build_fingerprints_equal"].is_null());
    assert_before_child(&summary); assert_soak_unreached(&summary); wrapper::assert_marker(&fixture);
}

#[test]
#[rustfmt::skip]
fn boundary_05_soak_drift_stays_primary_when_later_owner_cleanup_fails() {
    let temp = tempfile::tempdir().unwrap(); let fixture = wrapper::prepare(temp.path());
    append_module(&fixture, "CodexDiscordSoak.ProcessOwnership.psm1", r"
function Close-CodexSoakProcessOwner { param([object]$Owner) throw 'forced owner close failure' }
Export-ModuleMember -Function @('New-CodexSoakProcessOwner','Register-CodexSoakOwnedProcess','Stop-CodexSoakOwnedProcess','Close-CodexSoakProcessOwner')
");
    let child = wrapper::skip_build_command(&fixture, 2).spawn().unwrap();
    let (child, _) = wrapper::wait_for_memory(child, &fixture);
    fs::write(&fixture.source_file, b"bravo").unwrap();
    let output = wrapper::wait_for_output(child, Duration::from_secs(12)); assert_failed(&output);
    let summary = wrapper::summary(&fixture); let provenance = &summary["source_provenance"];
    assert_eq!(provenance["primary_failure"], json!({
        "stage": "source_fingerprint_soak_compare", "code": "source_changed_during_soak",
        "message": "Rust source changed during the soak boundary"
    }));
    assert_eq!(provenance["secondary_failures"], json!([{
        "stage": "cleanup", "code": "cleanup_failed", "message": "forced owner close failure"
    }]));
    assert_eq!(summary["operational_status"], "integrity_failed");
    assert_eq!(summary["child_exit_code"], 0);
    assert_eq!(summary["harness_provenance"]["process_exit_confirmed"], true);
    wrapper::assert_marker(&fixture);
}
