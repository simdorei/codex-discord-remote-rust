#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct AstEvent {
    kind: String,
    name: String,
    text: String,
    start: usize,
}

#[derive(Debug, Deserialize)]
struct AstEnvelope {
    errors: Vec<String>,
    events: Vec<AstEvent>,
}

fn ps_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn wrapper_ast() -> Vec<AstEvent> {
    let path = wrapper::repo_root().join("codex-discord-rust-soak.ps1");
    let bytes = fs::read(&path).unwrap();
    assert!(!bytes.starts_with(&[0xef, 0xbb, 0xbf]));
    std::str::from_utf8(&bytes).expect("wrapper must be strict UTF-8");
    let inspect = format!(
        r"$ErrorActionPreference='Stop'
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
    }} else {{
        $kind='member';$name=[string]$_.Member.Value
    }}
    [ordered]@{{kind=$kind;name=$name;text=$_.Extent.Text;start=$_.Extent.StartOffset}}
}})
[ordered]@{{errors=@($errors | ForEach-Object Message);events=$events}} |
    ConvertTo-Json -Depth 5 -Compress
",
        ps_literal(&path)
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &inspect])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "AST inspection failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut parsed: AstEnvelope = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        parsed.errors.is_empty(),
        "parse errors: {:?}",
        parsed.errors
    );
    parsed.events.sort_by_key(|event| event.start);
    parsed.events
}

fn named<'a>(events: &'a [AstEvent], kind: &str, name: &str) -> Vec<&'a AstEvent> {
    events
        .iter()
        .filter(|event| event.kind == kind && event.name.eq_ignore_ascii_case(name))
        .collect()
}

#[test]
fn w3_09_imports_and_build_fingerprint_gate_precede_process_start() {
    let events = wrapper_ast();
    let imports = named(&events, "command", "Import-Module");
    for module in [
        "SourceFingerprint",
        "Evidence",
        "WrapperRuntime",
        "WrapperEvidence",
    ] {
        let file = format!("codexdiscordsoak.{module}.psm1").to_ascii_lowercase();
        let imported = imports
            .iter()
            .any(|event| event.text.to_ascii_lowercase().contains(&file));
        assert!(imported, "missing CodexDiscordSoak.{module}.psm1 import");
    }

    let registration = named(&events, "command", "Register-CodexSoakOwnedProcess")
        .into_iter()
        .next()
        .expect("owned process registration");
    let process_start = named(&events, "member", "Start")
        .into_iter()
        .filter(|event| event.start < registration.start)
        .max_by_key(|event| event.start)
        .expect("process Start() before owned registration");
    let build_compare = named(&events, "command", "Test-CodexSoakSourceFingerprintEqual")
        .into_iter()
        .find(|event| event.start < process_start.start)
        .expect("source fingerprint comparison before process start");
    let snapshots_before_compare = named(&events, "command", "Get-CodexSoakSourceFingerprint")
        .into_iter()
        .filter(|event| event.start < build_compare.start)
        .count();
    assert!(snapshots_before_compare >= 2);
}

#[test]
fn w3_10_cleanup_precedes_soak_fingerprint_eligibility_and_atomic_publication() {
    let events = wrapper_ast();
    let first = |kind: &str, name: &str| {
        named(&events, kind, name)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    let stopped = first("command", "Stop-CodexSoakOwnedProcess");
    let provenance = first("command", "Complete-CodexSoakHarnessProvenance");
    assert!(stopped.start < provenance.start, "provenance preceded exit");

    let soak_after = named(&events, "command", "Get-CodexSoakSourceFingerprint")
        .into_iter()
        .find(|event| event.start > provenance.start)
        .expect("soak_after fingerprint after owned exit and provenance completion");
    let cleanup_end = [
        provenance.start,
        first("command", "Close-CodexSoakHarnessGuard").start,
        first("command", "Close-CodexSoakProcessOwner").start,
    ]
    .into_iter()
    .max()
    .unwrap();
    let soak_compare = named(&events, "command", "Test-CodexSoakSourceFingerprintEqual")
        .into_iter()
        .find(|event| event.start > soak_after.start)
        .expect("soak_after fingerprint comparison");
    let eligibility = first("command", "Get-CodexSoakFinalEligibility");
    assert!(cleanup_end < eligibility.start);
    assert!(soak_compare.start < eligibility.start);

    let summary = first("command", "New-CodexSoakSummaryV3");
    let publishers = named(&events, "command", "Publish-CodexSoakSummaryV3");
    assert_eq!(publishers.len(), 1, "exactly one final atomic publisher");
    let publisher = publishers[0];
    assert!(eligibility.start < summary.start && summary.start < publisher.start);
    assert!(
        named(&events, "member", "WriteAllText")
            .into_iter()
            .all(|event| !event.text.contains("$resultPath"))
    );
}

fn exact_process_live(pid: u64, started_at: &str) -> bool {
    let started_at = started_at.replace('\'', "''");
    let check = format!(
        "$p=Get-Process -Id {pid} -ErrorAction SilentlyContinue;\
         if($null -ne $p -and $p.StartTime.ToUniversalTime().ToString('o') -ceq '{started_at}'){{exit 0}};exit 7"
    );
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &check])
        .status()
        .unwrap()
        .success()
}

fn claims_passed_or_eligible(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(name, value)| {
            (name == "eligible" && value == &Value::Bool(true))
                || (name.ends_with("status")
                    && value
                        .as_str()
                        .is_some_and(|status| matches!(status, "passed" | "eligible")))
                || claims_passed_or_eligible(value)
        }),
        Value::Array(values) => values.iter().any(claims_passed_or_eligible),
        _ => false,
    }
}

#[test]
fn w3_10_live_three_second_harness_exposes_no_final_v3_success() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    let child = wrapper::skip_build_command(&fixture, 3)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (child, memory) = wrapper::wait_for_memory(child, &fixture);
    let pid = memory["harness_pid"].as_u64().unwrap();
    let started_at = memory["harness_started_at_utc"].as_str().unwrap();
    assert!(exact_process_live(pid, started_at));
    let premature = fs::read_dir(&fixture.evidence)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".summary.json") && !name.ends_with(".harness.summary.json")
        })
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .any(|value| {
            value["schema"] == "cdr.windows-soak.summary.v3" && claims_passed_or_eligible(&value)
        });
    assert!(
        !premature,
        "v3 success evidence existed while harness was live"
    );
    assert!(exact_process_live(pid, started_at));

    let output = wrapper::wait_for_output(child, Duration::from_secs(15));
    assert!(
        output.status.success(),
        "wrapper failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!exact_process_live(pid, started_at));
    assert!(wrapper::summary(&fixture)["schema"].is_string());
    wrapper::assert_marker(&fixture);
}
