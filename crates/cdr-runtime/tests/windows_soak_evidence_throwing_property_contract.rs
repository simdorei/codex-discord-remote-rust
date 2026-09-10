#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const HASH_A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const HASH_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
const HASH_C: &str = "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";

fn module_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/CodexDiscordSoak.Evidence.psm1")
}

const RUNNER: &str = r#"param([string]$ModulePath,[string]$Mode,[string]$InputPath)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$utf8=[Text.UTF8Encoding]::new($false,$true)
try {
    Import-Module -Name $ModulePath -Force
    if ($Mode -in @('eligibility_throw','fingerprint_throw')) {
        Add-Type -TypeDefinition @'
using System; using System.Collections; using System.Management.Automation;
public sealed class ExplodingDictionary : IDictionary {
    public static int Calls; private readonly string[] keys; private readonly string failureCode;
    public ExplodingDictionary(string[] keys, string failureCode) { this.keys = keys; this.failureCode = failureCode; }
    private Exception Failure() { var error = new InvalidOperationException("indexer exploded"); error.Data["CodexSoakFailureCode"] = failureCode; return error; }
    public ICollection Keys { get { Calls++; return new ExplodingKeys(); } }
    public ICollection Values { get { return new object[keys.Length]; } }
    public object this[object key] { get { Calls++; throw Failure(); } set { Calls++; throw Failure(); } }
    public bool IsFixedSize { get { return true; } } public bool IsReadOnly { get { return true; } }
    public int Count { get { return keys.Length; } } public bool IsSynchronized { get { return false; } }
    public object SyncRoot { get { return this; } }
    public void Add(object key, object value) { throw new NotSupportedException(); } public void Clear() { throw new NotSupportedException(); }
    public bool Contains(object key) { Calls++; return true; }
    public IDictionaryEnumerator GetEnumerator() { throw new InvalidOperationException(); } IEnumerator IEnumerable.GetEnumerator() { throw new InvalidOperationException(); }
    public void Remove(object key) { throw new NotSupportedException(); } public void CopyTo(Array array, int index) { throw new InvalidOperationException(); }
}
public sealed class ExplodingKeys : ICollection {
    public int Count { get { return 1; } } public bool IsSynchronized { get { return false; } }
    public object SyncRoot { get { return this; } } public void CopyTo(Array value, int index) { throw new InvalidOperationException(); }
    public IEnumerator GetEnumerator() { return new ExplodingEnumerator(); }
}
public sealed class ExplodingEnumerator : IEnumerator {
    public object Current { get { return null; } } public void Reset() { }
    public bool MoveNext() { ExplodingDictionary.Calls++; throw new PipelineStoppedException(); }
}
'@
    }
    if ($Mode -in @('eligibility_throw','eligibility_array')) {
        $checkNames=[string[]]@(
            'same_run_canonical_release_build','canonical_release_harness',
            'build_fingerprints_equal','soak_fingerprints_equal','harness_provenance_verified',
            'harness_process_exit_confirmed','child_exit_zero','harness_contract_passed',
            'event_stream_valid','minimum_duration_met','harness_duration_matches_request',
            'harness_elapsed_reached_request','canonical_warmup','sampling_interval_not_weakened',
            'slope_limit_not_weakened','regression_samples_met','memory_slope_passed',
            'disabled_marker_preserved','cleanup_clean'
        )
        if ($Mode -eq 'eligibility_throw') {
            $checks=[ExplodingDictionary]::new($checkNames,'attacker_code')
        } else {
            $checks=[ordered]@{}
            foreach ($name in $checkNames) { $checks[$name]=$true }
            $checks[$checkNames[0]]=[object[]]@($true)
            $checks=[pscustomobject]$checks
        }
        $result=Get-CodexSoakFinalEligibility -OperationalStatus passed -Checks $checks
        [ordered]@{unexpected=$result} | ConvertTo-Json -Depth 8 -Compress
        exit 0
    }
    $expected=[IO.File]::ReadAllText($InputPath,$utf8) | ConvertFrom-Json
    $actual=[IO.File]::ReadAllText($InputPath,$utf8) | ConvertFrom-Json
    $script:reads=0
    if ($Mode -eq 'fingerprint_throw') {
        $names=[string[]]@('schema','repo_root','aggregate_sha256','file_count','total_bytes','files')
        $expected=[ExplodingDictionary]::new($names,'attacker_code')
    } elseif ($Mode -eq 'schema_array') {
        $expected.schema=[object[]]@('cdr.rust-source-fingerprint.v1')
    } elseif ($Mode -eq 'bytes_array') {
        $expected.files[0].bytes=[object[]]@([long]1)
    } elseif ($Mode -eq 'files_list') {
        $list=[Collections.Generic.List[object]]::new()
        foreach ($row in $expected.files) { $list.Add($row) }
        $expected.files=$list
    } elseif ($Mode -eq 'files_rank2') { $matrix=[object[,]]::new(1,2); $matrix[0,0]=$expected.files[0]; $matrix[0,1]=$expected.files[1]; $expected.files=$matrix
    } elseif ($Mode -eq 'top_script') {
        $expected.PSObject.Properties.Remove('file_count')
        $expected | Add-Member ScriptProperty file_count {
            $script:reads++; throw [Management.Automation.PipelineStoppedException]::new()
        }
    } elseif ($Mode -eq 'row_script') {
        $expected.files[0].PSObject.Properties.Remove('bytes')
        $expected.files[0] | Add-Member ScriptProperty bytes {
            $script:reads++; throw [Management.Automation.PipelineStoppedException]::new()
        }
    }
    [ordered]@{
        equal=Test-CodexSoakSourceFingerprintEqual -Expected $expected -Actual $actual
    } | ConvertTo-Json -Compress
} catch {
    [ordered]@{
        code=$_.Exception.Data['CodexSoakFailureCode']; message=$_.Exception.Message
        calls=if ('ExplodingDictionary' -as [type]) { [ExplodingDictionary]::Calls } else { -1 }
        reads=$script:reads
    } | ConvertTo-Json -Compress
    exit 23
}
"#;

fn write_runner(path: &Path) {
    fs::write(path, RUNNER).unwrap();
}

fn fingerprint(root: &Path, file_count: usize) -> Value {
    let files = match file_count {
        0 => Vec::new(),
        1 => vec![json!({"path": "Cargo.lock", "bytes": 1, "sha256": HASH_B})],
        2 => vec![
            json!({"path": "Cargo.lock", "bytes": 1, "sha256": HASH_B}),
            json!({"path": "crates/core/lib.rs", "bytes": 2, "sha256": HASH_C}),
        ],
        _ => unreachable!(),
    };
    let total_bytes = files
        .iter()
        .map(|row| row["bytes"].as_u64().unwrap())
        .sum::<u64>();
    json!({
        "schema": "cdr.rust-source-fingerprint.v1",
        "repo_root": root,
        "aggregate_sha256": HASH_A,
        "file_count": file_count,
        "total_bytes": total_bytes,
        "files": files
    })
}

fn invoke(temp: &Path, mode: &str, input: &Value) -> Output {
    let runner = temp.join("invoke-throwing-property.ps1");
    let input_path = temp.join("property-input.json");
    write_runner(&runner);
    fs::write(&input_path, serde_json::to_vec(input).unwrap()).unwrap();
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(runner)
        .arg("-ModulePath")
        .arg(module_path())
        .arg("-Mode")
        .arg(mode)
        .arg("-InputPath")
        .arg(input_path)
        .output()
        .unwrap()
}

fn parse(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}); stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn evidence_properties_01_zero_and_one_file_arrays_remain_arrays() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    for file_count in [0, 1] {
        let output = invoke(temp.path(), "plain", &fingerprint(&root, file_count));
        assert!(output.status.success(), "result was: {}", parse(&output));
        assert_eq!(parse(&output)["equal"], true);
    }
}

#[test]
fn evidence_properties_02_script_properties_are_rejected_before_getters_run() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let input = fingerprint(&root, 2);
    for mode in ["top_script", "row_script"] {
        let output = invoke(temp.path(), mode, &input);
        assert_eq!(
            output.status.code(),
            Some(23),
            "result was: {}",
            parse(&output)
        );
        assert_eq!(parse(&output)["code"], "fingerprint_record_invalid");
        assert_eq!(parse(&output)["reads"], 0);
    }
}

#[test]
fn evidence_properties_03_public_apis_wrap_hostile_dictionary_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let input = fingerprint(&root, 2);
    for (mode, expected_code) in [
        ("fingerprint_throw", "fingerprint_record_invalid"),
        ("eligibility_throw", "eligibility_input_invalid"),
    ] {
        let output = invoke(temp.path(), mode, &input);
        assert_eq!(
            output.status.code(),
            Some(23),
            "result was: {}",
            parse(&output)
        );
        assert_eq!(parse(&output)["code"], expected_code);
        assert_eq!(parse(&output)["calls"], 0);
    }
}

#[test]
fn evidence_properties_04_scalar_arrays_and_non_array_files_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let input = fingerprint(&root, 2);
    for (mode, expected_code) in [
        ("schema_array", "fingerprint_record_invalid"),
        ("bytes_array", "fingerprint_record_invalid"),
        ("files_list", "fingerprint_record_invalid"),
        ("files_rank2", "fingerprint_record_invalid"),
        ("eligibility_array", "eligibility_input_invalid"),
    ] {
        let output = invoke(temp.path(), mode, &input);
        assert_eq!(
            output.status.code(),
            Some(23),
            "result was: {}",
            parse(&output)
        );
        assert_eq!(parse(&output)["code"], expected_code);
    }
}
