#![cfg(windows)]
use std::{path::Path, process::Command};

#[path = "native_process_observation_contract/cross_context.rs"]
mod cross_context;
#[path = "native_process_observation_contract/route_matrix.rs"]
mod route_matrix;

const RAW_OBSERVER_TRACE: &str = r"
& (Get-Module CdrNativeProcessObservation) {
    $script:CdrTraceOriginal=(Get-Command Get-CdrObservedProcessState -CommandType Function).ScriptBlock
    $script:CdrTraceLastCount=-1
    $script:CdrTraceNative=(Get-Command Invoke-CdrNative -CommandType Function).ScriptBlock
    $script:CdrTraceReady=(Get-Command Wait-CdrProcessObserverReady -CommandType Function).ScriptBlock
    $script:CdrTracePhase='before_readiness'; $script:CdrTraceCalls=0
    function script:Wait-CdrProcessObserverReady {
        param([string]$StartId,[string]$StopId,[TimeSpan]$Budget,$Observer)
        $script:CdrTracePhase='readiness'
        try { & $script:CdrTraceReady @PSBoundParameters }
        finally { $script:CdrTracePhase='after_readiness' }
    }
    function script:Invoke-CdrNative {
        param([string]$Executable,[string[]]$Arguments,[int]$TimeoutSeconds,[ref]$ProcessIdentity)
        $script:CdrTraceCalls++; $begin=[Diagnostics.Stopwatch]::GetTimestamp()
        try { & $script:CdrTraceNative @PSBoundParameters }
        finally {
            $identity=$ProcessIdentity.Value
            $record=[ordered]@{phase=$script:CdrTracePhase;call=$script:CdrTraceCalls;
                begin_tick=[string]$begin;end_tick=[string][Diagnostics.Stopwatch]::GetTimestamp();
                identity=$identity}
            [Console]::Error.WriteLine('RAW_OBSERVER_NATIVE '+($record|ConvertTo-Json -Depth 4 -Compress))
        }
    }
    function script:Get-CdrObservedProcessState {
        param([object[]]$Events,[string]$StartId,[string]$StopId,[uint32]$RootProcessId,
            [object[]]$Canaries,[object]$CommandProcess)
        $state=& $script:CdrTraceOriginal @PSBoundParameters
        $poll=[ordered]@{query_tick=[string][Diagnostics.Stopwatch]::GetTimestamp();phase=$script:CdrTracePhase;
            start_count=@($Events|Where-Object SourceIdentifier -eq $StartId).Count;
            stop_count=@($Events|Where-Object SourceIdentifier -eq $StopId).Count;event_count=$Events.Count}
        [Console]::Error.WriteLine('RAW_OBSERVER_POLL '+($poll|ConvertTo-Json -Compress))
        if($Events.Count -ne $script:CdrTraceLastCount) {
            $script:CdrTraceLastCount=$Events.Count
            $rows=@($Events | Select-Object -First 32 | ForEach-Object {
                $row=$_.SourceEventArgs.NewEvent
                $kind=if($_.SourceIdentifier -eq $StartId){'start'}elseif($_.SourceIdentifier -eq $StopId){'stop'}else{'other'}
                [pscustomobject]@{kind=$kind;pid=[uint32]$row.ProcessID;
                    parent=[uint32]$row.ParentProcessID;name=[string]$row.ProcessName;
                    time_created=[uint64]$row.TIME_CREATED}
            })
            $trace=[ordered]@{root_pid=$RootProcessId;event_count=$Events.Count;
                canaries=$Canaries;command=$CommandProcess;rows=$rows;state=$state}
            [Console]::Error.WriteLine('RAW_OBSERVER_TRACE '+($trace|ConvertTo-Json -Depth 6 -Compress))
        }
        return $state
    }
}
";

const READINESS_FIXTURE: &str = r"
$ErrorActionPreference='Stop'; Import-Module $env:CDR_OBSERVER -Force
& (Get-Module CdrNativeProcessObservation) {
    $script:CdrReadyCalls=[Collections.Generic.List[object]]::new()
    $script:CdrReadyCanaries=0; $script:CdrReadyPayloads=0; $script:CdrReadySubscriptions=0
    $script:CdrReadyStart=$null; $script:CdrReadyStop=$null
    function script:Start-CdrProcessObserver {
        param([string]$StartId,[string]$StopId)
        $script:CdrReadySubscriptions=2
        $script:CdrReadyStart=$StartId; $script:CdrReadyStop=$StopId
        [pscustomobject]@{start=$StartId;stop=$StopId}
    }
    function script:Stop-CdrProcessObserver {
        param($Observer)
        $script:CdrReadySubscriptions=0
        if($env:CDR_READY_MODE -like '*cleanup-failure'){throw 'injected observer cleanup failure'}
    }
    function script:Get-CdrProcessObserverEvents {
        param($Observer,[string]$StartId,[string]$StopId,[switch]$Final)
        Read-CdrReadinessFixture $StartId
        Read-CdrReadinessFixture $StopId
    }
    function script:Invoke-CdrNative {
        param([string]$Executable,[string[]]$Arguments,[int]$TimeoutSeconds,[ref]$ProcessIdentity)
        $name=[IO.Path]::GetFileName($Executable)
        if($name -eq 'fixture-payload.exe'){
            $script:CdrReadyPayloads++
            if($env:CDR_READY_MODE -eq 'payload-and-cleanup-failure'){throw 'injected payload failure'}
            if($env:CDR_READY_MODE -eq 'never' -or $script:CdrReadyCanaries -lt 2){throw 'payload started before provider readiness'}
        } elseif($name -eq 'cmd.exe'){$script:CdrReadyCanaries++}
        else{throw 'unexpected fixture executable'}
        $number=$script:CdrReadyCalls.Count+1
        $identity=[pscustomobject]@{pid=[uint32](5000+$number);name=$name;
            created_tick=[uint64]($number*10);exited_tick=[uint64]($number*10+3)}
        $script:CdrReadyCalls.Add($identity); $ProcessIdentity.Value=$identity
        if($name -eq 'fixture-payload.exe'){'payload-output'}
    }
    function script:Read-CdrReadinessFixture {
        [CmdletBinding()]param([string]$SourceIdentifier)
        if($env:CDR_READY_MODE -eq 'never'){return}
        foreach($identity in $script:CdrReadyCalls){
            # The first short probe predates provider readiness and has no events.
            if($identity.pid -eq 5001){continue}
            $tick=if($SourceIdentifier -eq $script:CdrReadyStart){$identity.created_tick+1}else{$identity.exited_tick+1}
            [pscustomobject]@{SourceIdentifier=$SourceIdentifier;SourceEventArgs=[pscustomobject]@{NewEvent=[pscustomobject]@{
                TIME_CREATED=[uint64]$tick;ProcessID=$identity.pid;ParentProcessID=[uint32]$PID;ProcessName=$identity.name}}}
        }
    }
}
$clock=[Diagnostics.Stopwatch]::StartNew(); $failure=$null; $published=@()
try {$published=@(Invoke-CdrObservedNativeCommand -Id readiness -Executable fixture-payload.exe -Arguments @() -OfflineFixture)}
catch {$failure=$_.Exception.Message}
$state=& (Get-Module CdrNativeProcessObservation) {
    [pscustomobject]@{canaries=$script:CdrReadyCanaries;payloads=$script:CdrReadyPayloads;subscriptions=$script:CdrReadySubscriptions}
}
if($state.subscriptions -ne 0){throw 'readiness subscriptions leaked'}
if($env:CDR_READY_MODE -eq 'never'){
    if($failure -notlike '*readiness timed out*command was not started*'){throw ('missing readiness failure: '+$failure)}
    if($state.payloads -ne 0 -or $published.Count -ne 0){throw 'unready observer executed or published the payload'}
    if($clock.Elapsed.TotalSeconds -lt 5 -or $clock.Elapsed.TotalSeconds -gt 20){throw 'readiness wait was not bounded'}
} elseif($env:CDR_READY_MODE -like '*cleanup-failure') {
    if($failure -notlike '*injected observer cleanup failure*'){throw ('missing cleanup failure: '+$failure)}
    if($env:CDR_READY_MODE -eq 'payload-and-cleanup-failure' -and $failure -notlike '*injected payload failure*'){throw 'original command error was overwritten'}
    if($state.payloads -ne 1 -or $published.Count -ne 0){throw 'cleanup failure published or replayed the payload'}
} else {
    if($failure){throw $failure}
    if($state.payloads -ne 1 -or $state.canaries -lt 3 -or $published.Count -ne 1){throw 'payload replay or incomplete readiness boundary'}
    if($published[0].status -ne 'completed' -or -not $published[0].command_process.observed){throw 'missing observed payload receipt'}
}
";

struct DeliveryDiagnostic {
    directory: tempfile::TempDir,
    children: Vec<std::process::Child>,
}

fn wait_diagnostic_child(
    child: &mut std::process::Child,
    budget: std::time::Duration,
) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "diagnostic child supervision timed out",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

impl Drop for DeliveryDiagnostic {
    fn drop(&mut self) {
        // Only these owned children and this unique temporary directory are touched.
        let _ = std::fs::write(self.directory.path().join("abort"), b"abort");
        for (index, child) in self.children.iter_mut().enumerate() {
            let role = if index == 0 { "launcher" } else { "observer" };
            if wait_diagnostic_child(child, std::time::Duration::from_secs(3)).is_err() {
                if let Err(error) = child.kill() {
                    eprintln!("EVENT_DELIVERY_DIAGNOSTIC_CLEANUP {role}: {error}");
                }
                if let Err(error) = wait_diagnostic_child(child, std::time::Duration::from_secs(1))
                {
                    eprintln!("EVENT_DELIVERY_DIAGNOSTIC_CLEANUP {role}: {error}");
                }
            }
            eprintln!(
                "EVENT_DELIVERY_DIAGNOSTIC_EXIT {role} pid={} {:?}",
                child.id(),
                child.try_wait()
            );
            for suffix in ["stdout", "stderr"] {
                match std::fs::read(self.directory.path().join(format!("{role}.{suffix}"))) {
                    Ok(bytes) => eprintln!("{}", String::from_utf8_lossy(&bytes)),
                    Err(error) => eprintln!("EVENT_DELIVERY_DIAGNOSTIC_LOG {role}: {error}"),
                }
            }
        }
    }
}

fn spawn_delivery_diagnostic(
    root: &Path,
    directory: &Path,
    role: &str,
) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("crates/cdr-runtime/tests/fixtures/native_process_event_delivery.ps1"))
        .args(["-Role", role, "-ControlDirectory"])
        .arg(directory)
        .env("CDR_DIAGNOSTIC_PARENT_PID", std::process::id().to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::fs::File::create(
            directory.join(format!("{role}.stdout")),
        )?)
        .stderr(std::fs::File::create(
            directory.join(format!("{role}.stderr")),
        )?)
        .creation_flags(0x0800_0000)
        .spawn()
}

fn collect_delivery_diagnostic(root: &Path, owned: &mut DeliveryDiagnostic) -> std::io::Result<()> {
    owned.children.push(spawn_delivery_diagnostic(
        root,
        owned.directory.path(),
        "launcher",
    )?);
    let ready = owned.directory.path().join("launcher-ready.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !ready.is_file() {
        if owned.children[0].try_wait()?.is_some() {
            return Err(std::io::Error::other(
                "diagnostic launcher exited before ready",
            ));
        }
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "diagnostic launcher did not become ready",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Both PowerShell hosts are children of this Rust process, not of each other.
    owned.children.push(spawn_delivery_diagnostic(
        root,
        owned.directory.path(),
        "observer",
    )?);
    wait_diagnostic_child(&mut owned.children[1], std::time::Duration::from_secs(20))?;
    wait_diagnostic_child(&mut owned.children[0], std::time::Duration::from_secs(3))
}

fn emit_event_delivery_diagnostic(root: &Path) {
    // Diagnostics never replace the original assertion or retry its payload.
    match tempfile::Builder::new()
        .prefix("cdr-observer-ab-")
        .tempdir()
    {
        Ok(directory) => {
            let mut owned = DeliveryDiagnostic {
                directory,
                children: Vec::new(),
            };
            if let Err(error) = collect_delivery_diagnostic(root, &mut owned) {
                eprintln!("EVENT_DELIVERY_DIAGNOSTIC_LAUNCH {error}");
            }
        }
        Err(error) => eprintln!("EVENT_DELIVERY_DIAGNOSTIC_LAUNCH {error}"),
    }
}

fn read_diagnostic_fault(case: &str) -> serde_json::Value {
    use std::os::windows::process::CommandExt;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("crates/cdr-runtime/tests/fixtures/native_process_event_delivery.ps1"))
        .args(["-Role", "cleanup-test", "-Fault", case])
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = stdout
        .lines()
        .find_map(|line| line.strip_prefix("EVENT_DELIVERY_DIAGNOSTIC "))
        .unwrap_or_else(|| panic!("missing final JSON: {stdout}; {:?}", output.stderr));
    serde_json::from_str(json).unwrap()
}

fn assert_diagnostic_cleanup_case(case: &str, failed_stage: Option<&str>) {
    let value = read_diagnostic_fault(case);
    let json = value.to_string();
    assert_eq!(value["test_case"], case);
    assert_eq!(value["post_cleanup_json"], true);
    assert_eq!(value["payload_retried"], false);
    assert!(value["error"].is_string());
    let stages = value["cleanup_stages"].as_array().unwrap();
    for stage in [
        "probe-dispose",
        "watcher-dispose",
        "unregister:cleanup-test-start",
        "remove:cleanup-test-start",
        "unregister:cleanup-test-stop",
        "remove:cleanup-test-stop",
    ] {
        assert!(
            stages.iter().any(|value| value == stage),
            "missing {stage}: {json}"
        );
    }
    let errors = value["cleanup_errors"].as_array().unwrap();
    if let Some(stage) = failed_stage {
        assert!(errors.iter().any(|value| value["stage"] == stage), "{json}");
        assert_eq!(value["error"], "injected primary failure");
    } else {
        assert!(errors.is_empty(), "{json}");
        assert!(stages.iter().any(|value| value == "probe-not-started"));
        assert!(!stages.iter().any(|value| value == "probe-status"));
    }
    eprintln!("DIAGNOSTIC_CLEANUP_CONTRACT {json}");
}

#[test]
fn diagnostic_failed_start_still_cleans_subscriptions_and_emits_json() {
    assert_diagnostic_cleanup_case("start-failure", None);
}

#[test]
fn diagnostic_child_cleanup_failure_does_not_skip_watchers_or_json() {
    assert_diagnostic_cleanup_case("child-cleanup", Some("probe-kill"));
}

#[test]
fn diagnostic_watcher_cleanup_failure_does_not_skip_subscriptions_or_json() {
    assert_diagnostic_cleanup_case("watcher-cleanup", Some("watcher-dispose"));
}
fn assert_readiness_fixture(mode: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            READINESS_FIXTURE,
        ])
        .env(
            "CDR_OBSERVER",
            root.join("scripts/CdrNativeProcessObservation.psm1"),
        )
        .env("CDR_READY_MODE", mode)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn observer_confirms_readiness_before_starting_payload_exactly_once() {
    assert_readiness_fixture("delayed");
}

#[test]
fn observer_readiness_timeout_never_starts_or_publishes_payload() {
    assert_readiness_fixture("never");
}

#[test]
fn observer_cleanup_failure_cannot_publish_or_replay_command() {
    assert_readiness_fixture("cleanup-failure");
}

#[test]
fn observer_preserves_command_error_when_cleanup_also_fails() {
    assert_readiness_fixture("payload-and-cleanup-failure");
}

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
fn etw_lifetimes_cannot_share_an_observed_instance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = r"
$ErrorActionPreference='Stop'
Import-Module $env:CDR_OBSERVER -Force
$failures=[Collections.Generic.List[string]]::new()
function Check([bool]$Value,[string]$Name) {
    if($Value){'PASS '+$Name}else{$failures.Add($Name);'FAIL '+$Name}
}
function Event($source,$time,$id) {
    [pscustomobject]@{SourceIdentifier=$source;SourceEventArgs=[pscustomobject]@{NewEvent=[pscustomobject]@{
        TIME_CREATED=$time;ProcessID=$id;ParentProcessID=42;ProcessName='cmd.exe'}}}
}
function Identity($id,$created,$exited) {
    [pscustomobject]@{pid=$id;name='cmd.exe';created_tick=$created;exited_tick=$exited}
}
$pre=Identity 51 1 2
$command=Identity 52 3 6
$post=Identity 52 7 8
$events=@((Event start 1 51),(Event stop 2 51),(Event start 7 52),(Event stop 8 52))
$arguments=@{StartId='start';StopId='stop';RootProcessId=42;Canaries=@($pre,$post);CommandProcess=$command}
$state=Get-CdrObservedProcessState -Events $events @arguments
Check (-not $state.ready -and -not $state.command_process_observed) 'later_canary_cannot_replace_missing_command_start'
$state=Get-CdrObservedProcessState -Events ($events+@(Event stop 6 52)) @arguments
Check (-not $state.ready -and -not $state.command_process_observed) 'orphan_command_stop_does_not_allow_later_lifecycle'
$overlap=Identity 52 3 8
$arguments.CommandProcess=$overlap
$state=Get-CdrObservedProcessState -Events $events @arguments
Check (-not $state.ready) 'overlapping_identities_cannot_share_one_lifecycle'
$arguments.CommandProcess=$command
$arguments.Canaries=@($pre,(Identity 53 7 8))
$valid=@((Event start 1 51),(Event stop 2 51),(Event start 3 52),(Event stop 6 52),(Event start 7 53),(Event stop 8 53))
$state=Get-CdrObservedProcessState -Events $valid @arguments
Check ($state.ready -and $state.command.start_tick -eq 3) 'distinct_lifetimes_are_accepted'
$arguments.Canaries=@($pre,$pre)
$state=Get-CdrObservedProcessState -Events $valid @arguments
Check (-not $state.ready) 'duplicate_canary_binding_is_rejected'
$arguments.Canaries=@($pre,(Identity 53 7 8))
$delayed=@($valid | Where-Object {$_.SourceEventArgs.NewEvent.ProcessID -ne 52})
$before=Get-CdrObservedProcessState -Events $delayed @arguments
$late=$delayed+@($valid | Where-Object {$_.SourceEventArgs.NewEvent.ProcessID -eq 52})
$after=Get-CdrObservedProcessState -Events $late @arguments
Check (-not $before.ready -and $after.ready -and $after.command.start_tick -eq 3 -and $after.command.stop_tick -eq 6) 'late_delivery_preserves_event_timestamps'
$arguments.CommandProcess=Identity 52 4 6
$state=Get-CdrObservedProcessState -Events $valid @arguments
Check (-not $state.ready) 'start_before_creation_is_rejected'
$arguments.CommandProcess=$command
$lateStop=@($valid | Where-Object {-not ($_.SourceIdentifier -eq 'stop' -and $_.SourceEventArgs.NewEvent.ProcessID -eq 52)})+@(Event stop 9 52)
$state=Get-CdrObservedProcessState -Events $lateStop @arguments
Check ($state.ready -and $state.command.stop_tick -eq 9) 'stop_after_exit_is_not_arbitrarily_rejected'
if($failures.Count){throw ('ETW identity regression failures: '+($failures -join ', '))}
'passed 8 ETW identity checks'
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
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("passed 8 ETW identity checks"));
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
$delayed=@($events | Where-Object {$_.SourceEventArgs.NewEvent.ProcessID -ne 60})
if((Get-CdrObservedProcessState -Events $delayed @arguments).ready){throw 'undelivered ETW command lifecycle passed'}
$late=$delayed+@($events | Where-Object {$_.SourceEventArgs.NewEvent.ProcessID -eq 60})
if(-not (Get-CdrObservedProcessState -Events $late @arguments).ready){throw 'valid late-delivered ETW events were rejected'}
$command.created_tick=88; $command.exited_tick=100
if((Get-CdrObservedProcessState -Events $events @arguments).ready){throw 'start 85 ticks before OS creation passed'}
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
fn utc_identity_clock_is_system_time_and_raw_diagnostic_clock_remains_qpc() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = r"
$ErrorActionPreference='Stop'
Add-Type -Path $env:CDR_KERNEL_SOURCE
$raw=[CdrKernelApi]::new()
$utc=[CdrKernelApi]::new('CDR-QA-ClockContract',$false)
try {
    if($raw.ClockContext -ne 1 -or $raw.TraceMode -ne 0x10001100){throw 'raw QPC configuration changed'}
    if($utc.ClockContext -ne 2 -or $utc.TraceMode -ne 0x10000100){throw 'UTC identity configuration is not system time'}
} finally {$raw.Dispose();$utc.Dispose()}
";
    let output = Command::new("powershell.exe")
        .env_remove("PSModulePath")
        .args(["-NoProfile", "-Command", script])
        .env(
            "CDR_KERNEL_SOURCE",
            root.join("crates/cdr-runtime/tests/fixtures/native_process_kernel_trace.cs"),
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
    # Synthetic post-command drain contract; real ETW readiness is tested separately.
    function script:Start-CdrProcessObserver { param($StartId,$StopId); [pscustomobject]@{} }
    function script:Stop-CdrProcessObserver { param($Observer) }
    function script:Get-CdrProcessObserverEvents { param($Observer,$StartId,$StopId,[switch]$Final) }
    $script:CdrMissingReadyCalls=0; $script:CdrMissingNativeCalls=0; $script:CdrMissingDrainCalls=0
    $script:CdrMissingCommand=$null
    $script:CdrMissingNative=(Get-Command Invoke-CdrNative -CommandType Function).ScriptBlock
    function script:Wait-CdrProcessObserverReady {
        param([string]$StartId,[string]$StopId,[TimeSpan]$Budget,$Observer)
        $script:CdrMissingReadyCalls++
        if($Budget.TotalSeconds -ne 5){throw 'fixture readiness budget changed'}
        [pscustomobject]@{canary=[pscustomobject]@{pid=[uint32]52;name='cmd.exe';created_tick=[uint64]1;exited_tick=[uint64]2};waited=[TimeSpan]::Zero}
    }
    function script:Invoke-CdrNative {
        param([string]$Executable,[string[]]$Arguments,[int]$TimeoutSeconds,[ref]$ProcessIdentity)
        $script:CdrMissingNativeCalls++
        $output=& $script:CdrMissingNative @PSBoundParameters
        if($script:CdrMissingNativeCalls -eq 1){$script:CdrMissingCommand=$ProcessIdentity.Value}
        return $output
    }
    function script:Get-CdrObservedProcessState {
        param([object[]]$Events,[string]$StartId,[string]$StopId,[uint32]$RootProcessId,
            [object[]]$Canaries,[object]$CommandProcess)
        if($Canaries.Count -ne 2 -or $null -eq $CommandProcess -or $script:CdrMissingNativeCalls -ne 2){throw 'missing-stop injection reached the wrong phase'}
        if($CommandProcess.pid -ne $script:CdrMissingCommand.pid){throw 'missing-stop command identity changed'}
        $script:CdrMissingDrainCalls++
        [pscustomobject]@{ready=$false;canary_observed=$true;command_process_observed=$true;remaining=@('52@3')}
    }
}

$clock=[Diagnostics.Stopwatch]::StartNew(); $failure=$null; $published=@()
try {$published=@(Invoke-CdrObservedNativeCommand -Id fixture -Executable cmd.exe -Arguments @('/d','/c','exit','0') -OfflineFixture)}
catch {$failure=$_.Exception.Message}
if($failure -notlike '*Process observation incomplete:*52@3*No passing record*'){throw 'missing original bounded failure'}
if($published.Count -ne 0){throw 'incomplete observation published a record'}
& (Get-Module CdrNativeProcessObservation) {
    if($script:CdrMissingReadyCalls -ne 1 -or $script:CdrMissingNativeCalls -ne 2 -or $script:CdrMissingDrainCalls -lt 1){throw 'expected one payload, one boundary, and the post-command drain injection'}
}
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
fn immediate_exit_with_empty_process_name_preserves_launch_identity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = r#"
$ErrorActionPreference='Stop'; Import-Module $env:CDR_NATIVE -Force
& (Get-Module CdrNativeProcess) {
    $body=(Get-Command Invoke-CdrNative).ScriptBlock.ToString()
    if(-not $body.Contains('$process.ProcessName')) { throw 'missing process-name fault injection point' }
    $body=$body.Replace('$process.ProcessName', "''")
    Set-Item Function:script:Invoke-CdrNative ([scriptblock]::Create($body))
    $identity=$null
    $null=Invoke-CdrNative -Executable cmd.exe -Arguments @('/d','/c','exit','0') -ProcessIdentity ([ref]$identity)
    if($identity.name -ne 'cmd.exe' -or $identity.pid -eq 0 -or
        $identity.created_tick -eq 0 -or $identity.exited_tick -le $identity.created_tick) {
        throw 'immediate exit lost launch identity'
    }
}
'passed'
"#;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env("CDR_NATIVE", root.join("scripts/CdrNativeProcess.psm1"))
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
    let script = format!(
        "$ErrorActionPreference='Stop'; Import-Module $env:CDR_OBSERVER -Force; \
        {RAW_OBSERVER_TRACE}
        Invoke-CdrObservedNativeCommand -Id fixture -Executable cmd.exe -Arguments @('/d','/c','exit',$env:CDR_CODE) -OfflineFixture | ConvertTo-Json -Compress"
    );
    for code in ["0", "7"] {
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script.as_str(),
            ])
            .env(
                "CDR_OBSERVER",
                root.join("scripts/CdrNativeProcessObservation.psm1"),
            )
            .env("CDR_CODE", code)
            .output()
            .unwrap();
        if code == "0" {
            if !output.status.success() {
                emit_event_delivery_diagnostic(&root);
            }
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

fn assert_diagnostic_publication_case(case: &str, primary: Option<&str>) {
    let value = read_diagnostic_fault(case);
    assert_eq!(value["test_case"], case);
    assert_eq!(value["diagnostic_only"], true);
    assert_eq!(value["post_cleanup_json"], true);
    assert_eq!(value["payload_retried"], false);
    assert!(value["cleanup_errors"].as_array().unwrap().is_empty());
    let publications = value["publication_errors"].as_array().unwrap();
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0]["stage"], "external-final.json");
    let message = publications[0]["error"].as_str().unwrap();
    assert!(message.contains("injected final publication I/O failure"));
    assert_eq!(value["error"].as_str(), Some(primary.unwrap_or(message)));
    let stages = value["cleanup_stages"].as_array().unwrap();
    for stage in [
        "watcher-dispose",
        "unregister:cleanup-test-start",
        "remove:cleanup-test-start",
        "unregister:cleanup-test-stop",
        "remove:cleanup-test-stop",
    ] {
        assert!(
            stages.iter().any(|value| value == stage),
            "missing {stage}: {value}"
        );
    }
    eprintln!("DIAGNOSTIC_PUBLICATION_CONTRACT {value}");
}

#[test]
fn diagnostic_publication_failure_preserves_the_primary_error() {
    assert_diagnostic_publication_case(
        "publication-failure",
        Some("injected coordination timeout"),
    );
}

#[test]
fn diagnostic_publication_failure_without_primary_remains_a_failure() {
    assert_diagnostic_publication_case("publication-only", None);
}

#[test]
fn native_creation_mode_diagnostic_retains_raw_events_without_claiming_native_pass() {
    use std::os::windows::process::CommandExt;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("crates/cdr-runtime/tests/fixtures/native_process_creation_matrix.ps1"))
        .env("CDR_MATRIX_PARENT_PID", std::process::id().to_string())
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    eprintln!(
        "NATIVE_CREATION_MODE_MATRIX {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.status.success(),
        "matrix collection failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["diagnostic_only"], true);
    assert_eq!(value["native_gate_pass"], false);
    assert_eq!(value["comparison_valid"], true);
    assert_eq!(value["acquisition_budget_ms"], 5000);
    assert_eq!(value["subscriptions_registered"], 2);
    assert!(value["error"].is_null());
    assert!(value["cleanup_errors"].as_array().unwrap().is_empty());
    let probes = value["probes"].as_array().unwrap();
    assert_eq!(probes.len(), 2);
    assert_eq!(probes[0]["create_no_window"], true);
    assert_eq!(probes[1]["create_no_window"], false);
    assert_ne!(probes[0]["process_id"], probes[1]["process_id"]);
    for probe in probes {
        assert_eq!(probe["creator_pid"], value["host_pid"]);
        assert_eq!(probe["window_style"], "Hidden");
        assert_eq!(probe["ready"], true);
        assert_eq!(probe["released"], true);
        assert_eq!(probe["exit_confirmed"], true);
        assert_eq!(probe["exit_code"], 0);
        assert_eq!(probe["termination_requested"], false);
    }
    assert_eq!(
        value["raw_row_count"].as_u64().unwrap(),
        u64::try_from(value["raw_rows"].as_array().unwrap().len()).unwrap()
    );
}

fn assert_matrix_cleanup_fault(case: &str, expected_stages: &[&str], confirmed: bool) {
    use std::os::windows::process::CommandExt;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("crates/cdr-runtime/tests/fixtures/native_process_creation_matrix.ps1"))
        .args(["-CleanupTestCase", case])
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cleanup fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    eprintln!("MATRIX_CLEANUP_CONTRACT {value}");
    assert_eq!(value["synthetic_cleanup_contract"], true);
    assert_eq!(value["diagnostic_only"], true);
    assert_eq!(value["native_gate_pass"], false);
    assert_eq!(value["comparison_valid"], false);
    assert_eq!(value["fault_case"], case);
    assert_eq!(value["error"], "injected primary collection failure");
    assert_eq!(value["exit_confirmed"], confirmed);
    assert_eq!(value["termination_requested"], true);
    assert_eq!(value["safe_for_follow_up"], false);
    assert_eq!(
        value["trace"],
        serde_json::json!(["stdin-close", "wait:1", "kill", "wait:2", "dispose"])
    );
    let stages: Vec<_> = value["cleanup_errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| error["stage"].as_str().unwrap())
        .collect();
    assert_eq!(stages, expected_stages);
}

#[test]
fn matrix_cleanup_close_error_does_not_skip_kill_or_exit_confirmation() {
    assert_matrix_cleanup_fault("close-error", &["probe-stdin-close"], true);
}

#[test]
fn matrix_cleanup_initial_wait_error_still_attempts_kill_and_final_wait() {
    assert_matrix_cleanup_fault("initial-wait-error", &["probe-wait-initial"], true);
}

#[test]
fn matrix_cleanup_kill_error_preserves_exit_uncertainty() {
    assert_matrix_cleanup_fault("kill-error", &["probe-kill", "probe-exit-confirm"], false);
}

#[test]
fn matrix_cleanup_final_wait_false_never_claims_confirmed_exit() {
    assert_matrix_cleanup_fault("final-wait-false", &["probe-exit-confirm"], false);
}
