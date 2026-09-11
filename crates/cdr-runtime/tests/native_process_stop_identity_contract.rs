#![cfg(windows)]
use std::{path::Path, process::Command};

const EVENTS: &str = r"
$ErrorActionPreference='Stop'
Import-Module $env:CDR_OBSERVER -Force
function Event($source,$time,$id,$parent,$name) {
    [pscustomobject]@{SourceIdentifier=$source;SourceEventArgs=[pscustomobject]@{NewEvent=[pscustomobject]@{
        TIME_CREATED=$time;ProcessID=$id;ParentProcessID=$parent;ProcessName=$name}}}
}
$events=@((Event start 1 51 42 cmd.exe),(Event stop 2 51 0 cmd.exe),
    (Event start 3 52 42 worker.exe),(Event start 7 53 42 cmd.exe),(Event stop 8 53 0 cmd.exe))
$canaries=@([pscustomobject]@{pid=51;name='cmd.exe';created_tick=1;exited_tick=2},[pscustomobject]@{pid=53;name='cmd.exe';created_tick=7;exited_tick=8})
$command=[pscustomobject]@{pid=52;name='worker.exe';created_tick=3;exited_tick=6}
$arguments=@{StartId='start';StopId='stop';RootProcessId=42;Canaries=$canaries;CommandProcess=$command}
";

fn run_case(script: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env(
            "CDR_OBSERVER",
            root.join("scripts/CdrNativeProcessObservation.psm1"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn foreign_stop_without_its_start_cannot_complete_an_owned_instance() {
    run_case(&format!(
        "{EVENTS}{}",
        r"
$events+=Event stop 10 52 0 python.exe
$state=Get-CdrObservedProcessState -Events $events @arguments
if($state.ready -or $state.remaining.Count -ne 1 -or $state.remaining[0] -ne '52@3'){throw 'S1: foreign stop falsely completed old owned PID'}
$events+=Event start 9 52 999 python.exe
$state=Get-CdrObservedProcessState -Events $events @arguments
if($state.ready -or $state.remaining.Count -ne 1){throw 'S1: late foreign start erased the old obligation'}
$events+=Event stop 6 52 0 worker.exe
$state=Get-CdrObservedProcessState -Events $events @arguments
if(-not $state.ready -or $state.command.stop_tick -ne 6 -or $state.owned.Count -ne 3){throw 'S1: only the real late stop may complete the old instance'}
"
    ));
}

#[test]
fn stop_name_compatibility_is_bounded_and_known_foreign_parents_are_rejected() {
    run_case(&format!(
        "{EVENTS}{}",
        r"
$cases=@(
    @('worker.exe','python.exe',0,$false), @('worker.exe','worker',0,$false),
    @('worker.exe','',0,$false), @('worker.exe','WORKER.EXE',0,$true),
    @('worker.exe','worker.exe',999,$false), @('worker.exe','worker.exe',42,$true),
    @('cdr-pro-helper.exe','cdr-pro-helper',0,$true),
    @('native_process_observer.exe','native_process',0,$true),
    @('cdr-pro-helper.exe','cdr-pro-helpe',0,$false),
    @('cdr-pro-helper.exe','cdr-pro-helperX',0,$false),
    @('native_process_nonascii한.exe','native_process',0,$false))
foreach($case in $cases) {
    $command.name=$case[0]; $events[2].SourceEventArgs.NewEvent.ProcessName=$case[0]
    $state=Get-CdrObservedProcessState -Events ($events+@(Event stop 6 52 $case[2] $case[1])) @arguments
    if($state.ready -ne $case[3]){throw ('S2: wrong stop compatibility for '+($case -join '/'))}
    if(-not $case[3] -and ($state.remaining.Count -ne 1 -or $state.remaining[0] -ne '52@3')){throw 'S2: rejected stop erased the pending identity'}
}
"
    ));
}

const PRODUCER: &str = r"
$ErrorActionPreference='Stop'
Import-Module $env:CDR_OBSERVER -Force
& (Get-Module CdrNativeProcessObservation) {
    $script:launches=0; $script:startReads=0; $script:stopReads=0
    $script:subscriptions=[Collections.Generic.List[string]]::new(); $script:subscriptions.Add('unrelated')
    $script:removed=[Collections.Generic.List[string]]::new()
    function script:Register-WmiEvent { param($Class,$SourceIdentifier); $script:subscriptions.Add($SourceIdentifier) }
    function script:Unregister-Event { param($SourceIdentifier); $null=$script:subscriptions.Remove($SourceIdentifier) }
    function script:Remove-Event {
        [CmdletBinding()]param([Parameter(ValueFromPipeline)]$InputObject)
        process {$script:removed.Add([string]$InputObject.SourceIdentifier)}
    }
    function script:Invoke-CdrNative {
        param($Executable,$Arguments,$TimeoutSeconds,[ref]$ProcessIdentity)
        $script:launches++
        $identity=switch($script:launches) {
            1 {[pscustomobject]@{pid=51;name='cmd.exe';created_tick=1;exited_tick=2}}
            2 {[pscustomobject]@{pid=52;name='worker.exe';created_tick=3;exited_tick=6}}
            3 {[pscustomobject]@{pid=53;name='cmd.exe';created_tick=7;exited_tick=8}}
            default {throw 'unexpected extra native execution'}
        }
        $ProcessIdentity.Value=$identity
        'fixture stdout'
    }
    function script:FixtureEvent($source,$time,$id,$parent,$name) {
        [pscustomobject]@{SourceIdentifier=$source;SourceEventArgs=[pscustomobject]@{NewEvent=[pscustomobject]@{
            TIME_CREATED=$time;ProcessID=$id;ParentProcessID=$parent;ProcessName=$name}}}
    }
    function script:Get-Event {
        [CmdletBinding()]param([string]$SourceIdentifier)
        if($SourceIdentifier -like 'CdrObservedStart-*') {
            $script:startReads++
            FixtureEvent $SourceIdentifier 1 51 $PID cmd.exe
            FixtureEvent $SourceIdentifier 3 52 $PID worker.exe
            FixtureEvent $SourceIdentifier 7 53 $PID cmd.exe
            if($script:startReads -gt 1){FixtureEvent $SourceIdentifier 9 52 999 python.exe}
        } elseif($SourceIdentifier -like 'CdrObservedStop-*') {
            $script:stopReads++
            FixtureEvent $SourceIdentifier 2 51 0 cmd.exe
            FixtureEvent $SourceIdentifier 8 53 0 cmd.exe
            FixtureEvent $SourceIdentifier 10 52 0 python.exe
            if($script:startReads -gt 1 -and $script:lateCorrectStop){FixtureEvent $SourceIdentifier 6 52 0 worker.exe}
        } else {throw 'queried unrelated events'}
    }
}
";

#[test]
fn split_queries_with_missing_owned_stop_time_out_without_a_record_and_clean_up() {
    run_case(&format!(
        "{PRODUCER}{}",
        r"
& (Get-Module CdrNativeProcessObservation) {$script:lateCorrectStop=$false}
$clock=[Diagnostics.Stopwatch]::StartNew(); $failure=$null; $published=@()
try {$published=@(Invoke-CdrObservedNativeCommand -Id fixture -Executable worker.exe -Arguments @() -OfflineFixture)}
catch {$failure=$_.Exception.Message}
if($published.Count -ne 0){throw 'S3: split-query foreign stop published a false success record'}
if($failure -notlike '*Process observation incomplete:*52@3*No passing record*'){throw ('S3: wrong failure: '+$failure)}
if($clock.Elapsed.TotalSeconds -lt 5 -or $clock.Elapsed.TotalSeconds -gt 15){throw 'S3: incomplete observation did not use the bounded drain'}
& (Get-Module CdrNativeProcessObservation) {
    if($script:launches -ne 3 -or $script:startReads -lt 2 -or $script:stopReads -lt 2){throw 'S3: expected actual producer collection and retry'}
    if($script:subscriptions.Count -ne 1 -or $script:subscriptions[0] -ne 'unrelated'){throw 'S3: subscription cleanup changed unrelated state or leaked'}
    $cleaned=@($script:removed | Sort-Object -Unique)
    if($cleaned.Count -ne 2 -or $cleaned[0] -notlike 'CdrObservedStart-*' -or $cleaned[1] -notlike 'CdrObservedStop-*'){throw 'S3: missing owned event cleanup'}
}
"
    ));
}

#[test]
fn split_queries_wait_for_the_real_late_stop_before_publishing_once() {
    run_case(&format!(
        "{PRODUCER}{}",
        r"
& (Get-Module CdrNativeProcessObservation) {$script:lateCorrectStop=$true}
$published=@(Invoke-CdrObservedNativeCommand -Id fixture -Executable worker.exe -Arguments @() -OfflineFixture)
if($published.Count -ne 1 -or $published[0].status -ne 'completed'){throw 'S4: expected exactly one completed record'}
$actualStop=([DateTime]$published[0].command_process.stop_event_at).ToUniversalTime().ToFileTimeUtc()
if($actualStop -ne 6){throw 'S4: record used the foreign stop instead of the actual late stop'}
if($published[0].owned_process_starts -ne 3 -or $published[0].recognized_python_process_count -ne 0){throw 'S4: foreign process inherited ownership'}
& (Get-Module CdrNativeProcessObservation) {
    if($script:startReads -lt 2 -or $script:stopReads -lt 2 -or $script:launches -ne 3){throw 'S4: producer completed early or reran the native command'}
    if($script:subscriptions.Count -ne 1 -or $script:subscriptions[0] -ne 'unrelated'){throw 'S4: cleanup did not preserve the unrelated subscription'}
    if(@($script:removed | Sort-Object -Unique).Count -ne 2){throw 'S4: owned event cleanup missing'}
}
"
    ));
}
