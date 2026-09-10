#![cfg(windows)]

#[path = "support/windows_soak_wrapper.rs"]
mod wrapper;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

const FORCED_FAILURE: &str = "forced registration failure";
const MISSING_NATIVE_HANDLE: &str = "Owned soak child had no duplicated native process handle";
const RETAINED_PROCESS_BARRIER: &str =
    "Using retained original Process for the fail-closed exit barrier";
const SPURIOUS_IDENTITY_LOSS: &str =
    "Requested PID/start did not match retained exact process identity";

fn run(mut command: Command) -> Output {
    let child = command.spawn().unwrap();
    wrapper::wait_for_output(child, Duration::from_secs(20))
}

fn inject_registration_failure(fixture: &wrapper::Fixture) {
    let path = fixture
        .root
        .join("scripts/CodexDiscordSoak.ProcessOwnership.psm1");
    let mut module = fs::read_to_string(&path).unwrap();
    module.push_str(
        r"
function Register-CodexSoakOwnedProcess {
    param([Parameter(Mandatory)][object]$Owner, [Parameter(Mandatory)][Diagnostics.Process]$Process)
    $Owner.Process = $Process
    throw 'forced registration failure'
}
Export-ModuleMember -Function 'Register-CodexSoakOwnedProcess'
",
    );
    fs::write(path, module).unwrap();
}

fn comparable_path(value: &str) -> String {
    value
        .strip_prefix(r"\\?\")
        .unwrap_or(value)
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn assert_exact_process_is_not_live(pid: u64, start_ticks: i64) {
    let script = r"
param([int]$TargetProcessId, [long]$ExpectedStartTicks)
$process = Get-Process -Id $TargetProcessId -ErrorAction SilentlyContinue
if ($null -eq $process) { exit 0 }
try { $actual = [long]$process.StartTime.ToUniversalTime().Ticks }
catch { Write-Error $_.Exception.Message; exit 2 }
if ($actual -eq $ExpectedStartTicks) { exit 1 }
exit 0
";
    let command = format!("& {{ {script} }} {pid} {start_ticks}");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "exact launched child remained live or could not be checked (exit {:?})\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn registration_failure_preserves_launched_child_identity_output_and_exit_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = wrapper::prepare(temp.path());
    inject_registration_failure(&fixture);

    let output = run(wrapper::skip_build_command(&fixture, 1));
    assert!(
        !output.status.success(),
        "forced registration failure unexpectedly passed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let summary = wrapper::summary(&fixture);
    assert_eq!(summary["schema"], "cdr.windows-soak.summary.v3");
    assert!(summary.get("status").is_none());
    assert_eq!(summary["operational_status"], "runtime_failed");
    assert_eq!(summary["final_eligibility"]["status"], "failed");
    assert_eq!(summary["final_eligibility"]["eligible"], false);

    let source = &summary["source_provenance"];
    let primary = &source["primary_failure"];
    assert_eq!(primary["stage"], "start");
    assert_eq!(primary["code"], "start_failed");
    assert_eq!(primary["message"], FORCED_FAILURE);

    let pid = summary["harness_pid"].as_u64().expect("exact harness PID");
    assert!(pid > 0);
    let started_at = summary["harness_started_at_utc"]
        .as_str()
        .filter(|value| !value.is_empty())
        .expect("exact harness start time");
    assert!(summary["child_exit_code"].as_i64().is_some());

    let provenance = &summary["harness_provenance"];
    assert_eq!(provenance["pid"], summary["harness_pid"]);
    assert_eq!(provenance["process_started_at_utc"], started_at);
    let start_ticks = provenance["process_start_ticks"]
        .as_i64()
        .expect("exact harness start ticks");
    assert!(start_ticks > 0);
    assert_eq!(provenance["process_exit_confirmed"], true);
    assert_eq!(provenance["verified"], true);

    let actual_path = provenance["actual_process_image_path"]
        .as_str()
        .filter(|value| !value.is_empty())
        .expect("actual process image path");
    let canonical_path = fs::canonicalize(&fixture.debug_harness).unwrap();
    assert_eq!(
        comparable_path(actual_path),
        comparable_path(&canonical_path.to_string_lossy())
    );
    assert_eq!(
        comparable_path(actual_path),
        comparable_path(provenance["canonical_path"].as_str().unwrap())
    );

    for name in ["stdout", "stderr"] {
        let path = summary["outputs"][name]
            .as_str()
            .unwrap_or_else(|| panic!("{name} output path"));
        assert!(
            Path::new(path).is_file(),
            "{name} output file missing: {path}"
        );
    }

    let encoded = serde_json::to_string(&summary).unwrap();
    assert!(!encoded.contains("harness_changed_after_launch"));
    assert!(!encoded.contains(SPURIOUS_IDENTITY_LOSS));
    let secondary = source["secondary_failures"]
        .as_array()
        .expect("secondary failure ledger");
    for failure in secondary {
        assert_eq!(failure["stage"], "cleanup");
        assert_eq!(failure["code"], "cleanup_failed");
        let message = failure["message"].as_str().expect("cleanup message");
        assert!(
            matches!(message, MISSING_NATIVE_HANDLE | RETAINED_PROCESS_BARRIER),
            "unexpected cleanup secondary: {message}"
        );
    }

    assert_exact_process_is_not_live(pid, start_ticks);
    wrapper::assert_marker(&fixture);
}
