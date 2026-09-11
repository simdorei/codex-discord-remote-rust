#![cfg(windows)]
use std::{path::Path, process::Command};

#[test]
fn boundary_canaries_do_not_hide_a_delayed_owned_stop_or_foreign_pid_reuse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = r"
$ErrorActionPreference='Stop'
Import-Module $env:CDR_OBSERVER -Force
function Event($source,$time,$id,$parent,$name) {
    [pscustomobject]@{SourceIdentifier=$source;SourceEventArgs=[pscustomobject]@{NewEvent=[pscustomobject]@{
        TIME_CREATED=$time;ProcessID=$id;ParentProcessID=$parent;ProcessName=$name}}}
}
$events=@((Event start 1 51 42 cmd.exe),(Event stop 2 51 42 cmd.exe),
    (Event start 3 52 42 worker.exe),(Event start 4 53 42 cmd.exe),(Event stop 5 53 42 cmd.exe))
$canaries=@([pscustomobject]@{pid=51;name='cmd.exe';created_tick=1;exited_tick=2},[pscustomobject]@{pid=53;name='cmd.exe';created_tick=4;exited_tick=5})
$command=[pscustomobject]@{pid=52;name='worker.exe';created_tick=3;exited_tick=6}
$arguments=@{StartId='start';StopId='stop';RootProcessId=42;Canaries=$canaries;CommandProcess=$command}
$before=Get-CdrObservedProcessState -Events $events @arguments
if($before.ready -or -not $before.canary_observed -or $before.remaining.Count -ne 1){throw 'delayed owned stop incorrectly passed'}
$events += Event start 7 52 999 python.exe
$missingStop=Get-CdrObservedProcessState -Events $events @arguments
if($missingStop.ready -or $missingStop.remaining.Count -ne 1){throw 'foreign PID reuse erased the pending owned stop'}
$events += Event stop 8 52 999 python.exe
$foreignStop=Get-CdrObservedProcessState -Events $events @arguments
if($foreignStop.ready -or $foreignStop.remaining.Count -ne 1){throw 'foreign stop completed the old owned instance'}
$events += Event stop 6 52 42 worker.exe
$after=Get-CdrObservedProcessState -Events $events @arguments
if(-not $after.ready -or $after.owned.Count -ne 3 -or @($after.owned | Where-Object name -eq python.exe).Count){throw 'foreign PID reuse inherited ownership'}
'passed'
";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env(
            "CDR_OBSERVER",
            root.join("scripts/CdrNativeProcessObservation.psm1"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("passed"));
}

#[test]
fn canaries_cannot_replace_the_missing_command_start_even_when_its_child_is_seen() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = r"
$ErrorActionPreference='Stop'
Import-Module $env:CDR_OBSERVER -Force
function Event($source,$time,$id,$parent,$name) {
    [pscustomobject]@{SourceIdentifier=$source;SourceEventArgs=[pscustomobject]@{NewEvent=[pscustomobject]@{
        TIME_CREATED=$time;ProcessID=$id;ParentProcessID=$parent;ProcessName=$name}}}
}
$events=@((Event start 1 51 42 cmd.exe),(Event stop 2 51 42 cmd.exe),
    (Event start 7 53 42 cmd.exe),(Event stop 8 53 42 cmd.exe))
$canaries=@([pscustomobject]@{pid=51;name='cmd.exe';created_tick=1;exited_tick=2},[pscustomobject]@{pid=53;name='cmd.exe';created_tick=7;exited_tick=8})
$command=[pscustomobject]@{pid=60;name='cargo.exe';created_tick=3;exited_tick=6}
$arguments=@{StartId='start';StopId='stop';RootProcessId=42;Canaries=$canaries;CommandProcess=$command}
foreach($includeChild in @($false,$true)) {
    $inputEvents=$events
    if($includeChild){$inputEvents+=@((Event start 4 61 60 python.exe),(Event stop 5 61 60 python.exe),(Event stop 6 60 42 cargo.exe))}
    $state=Get-CdrObservedProcessState -Events $inputEvents @arguments
    if($state.ready){throw ('missing command start incorrectly passed; child='+$includeChild)}
}
$events+=@((Event start 3 60 42 cargo.exe),(Event stop 6 60 42 cargo.exe))
$valid=Get-CdrObservedProcessState -Events $events @arguments
if(-not $valid.ready -or -not $valid.command_process_observed){throw 'matching actual command instance was rejected'}
$ambiguous=$events+@((Event start 9 60 42 cargo.exe),(Event stop 10 60 42 cargo.exe))
if((Get-CdrObservedProcessState -Events $ambiguous @arguments).ready){throw 'ambiguous owned PID reuse passed'}
$late=$events | ForEach-Object {$_.SourceEventArgs.NewEvent.TIME_CREATED+=20;$_}
if(-not (Get-CdrObservedProcessState -Events $late @arguments).ready){throw 'valid WMI notifications after native exit were rejected'}
$events | ForEach-Object {$_.SourceEventArgs.NewEvent.TIME_CREATED-=20}
$command.created_tick=7; $command.exited_tick=9
if((Get-CdrObservedProcessState -Events $events @arguments).ready){throw 'same PID/name from a different creation lifetime passed'}
";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env(
            "CDR_OBSERVER",
            root.join("scripts/CdrNativeProcessObservation.psm1"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn missing_stop_exhausts_the_bounded_drain_without_publishing_a_record() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = r"
$ErrorActionPreference='Stop'
Import-Module $env:CDR_OBSERVER -Force
& (Get-Module CdrNativeProcessObservation) {
    function script:Get-CdrObservedProcessState {
        [pscustomobject]@{ready=$false;canary_observed=$true;command_process_observed=$true;remaining=@('52@3')}
    }
}

$clock=[Diagnostics.Stopwatch]::StartNew(); $failure=$null; $published=@()
try {$published=@(Invoke-CdrObservedNativeCommand -Id fixture -Executable cmd.exe -Arguments @('/d','/c','exit','0') -OfflineFixture)}
catch {$failure=$_.Exception.Message}
if($failure -notlike '*Process observation incomplete:*52@3*No passing record*'){throw 'missing original bounded failure'}
if($published.Count -ne 0){throw 'incomplete observation published a record'}
if($clock.Elapsed.TotalSeconds -lt 5 -or $clock.Elapsed.TotalSeconds -gt 20){throw 'drain was not bounded'}
if(@(Get-EventSubscriber | Where-Object SourceIdentifier -like 'CdrObserved*').Count){throw 'observer subscriptions leaked'}
'passed'
";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env(
            "CDR_OBSERVER",
            root.join("scripts/CdrNativeProcessObservation.psm1"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("passed"));
}

#[test]
fn real_executable_alias_and_truncated_stop_names_do_not_lose_process_identity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = r"
$ErrorActionPreference='Stop'; Import-Module $env:CDR_OBSERVER -Force
foreach($command in @(@{exe='cargo';args=@('--version')},@{exe=$env:CDR_LONG_COMMAND;args=@('--list')})) {
    $record=Invoke-CdrObservedNativeCommand -Id fixture -Executable $command.exe -Arguments $command.args -OfflineFixture
    if($record.status -ne 'completed' -or -not $record.command_process.observed){throw 'actual executable identity was lost'}
}
'passed'
";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env(
            "CDR_OBSERVER",
            root.join("scripts/CdrNativeProcessObservation.psm1"),
        )
        .env("CDR_LONG_COMMAND", std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("passed"));
}

#[test]
fn actual_boundary_canaries_are_required_and_command_failure_has_no_pass_record() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = "$ErrorActionPreference='Stop'; Import-Module $env:CDR_OBSERVER -Force; \
        Invoke-CdrObservedNativeCommand -Id fixture -Executable cmd.exe -Arguments @('/d','/c','exit',$env:CDR_CODE) -OfflineFixture | ConvertTo-Json -Compress";
    for code in ["0", "7"] {
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ])
            .env(
                "CDR_OBSERVER",
                root.join("scripts/CdrNativeProcessObservation.psm1"),
            )
            .env("CDR_CODE", code)
            .output()
            .unwrap();
        if code == "0" {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let record: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(record["status"], "completed");
            assert_eq!(record["canary_observed"], true);
            assert!(record["owned_process_starts"].as_u64().unwrap() >= 3);
            assert_eq!(record["command_process"]["observed"], true);
            assert!(record["command_process"]["pid"].as_u64().unwrap() > 0);
            assert_eq!(record["command_process"]["name"], "cmd.exe");
            assert_eq!(record["recognized_python_process_count"], 0);
            assert_eq!(record["exit_code"], 0);
        } else {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("exit code 7"));
            assert!(!String::from_utf8_lossy(&output.stdout).contains("completed"));
        }
    }
}
