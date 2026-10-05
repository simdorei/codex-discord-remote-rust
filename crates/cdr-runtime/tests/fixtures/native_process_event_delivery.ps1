# Diagnostic only. Never substitutes for a passing observation record or replays a payload.
param(
    [ValidateSet('observer','launcher','cleanup-test')][string]$Role='observer',
    [string]$ControlDirectory,
    [ValidateSet('start-failure','child-cleanup','watcher-cleanup','publication-failure','publication-only')][string]$Fault='start-failure'
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Set-StrictMode -Version Latest

function Invoke-DeliveryCleanupStep {
    param([string]$Stage,[scriptblock]$Action,$Cleanup)
    $Cleanup.stages.Add($Stage)
    try { & $Action | Out-Null }
    catch { $Cleanup.errors.Add([pscustomobject]@{stage=$Stage;error=$_.Exception.Message}) }
}

function Complete-DeliveryCleanup {
    param($Context,$Tap,[string[]]$SourceIds,$Cleanup)
    if($null -ne $Context.probe) {
        if($Context.started) {
            $state=@{running=$true;exited=$false}
            Invoke-DeliveryCleanupStep 'probe-status' {$state.running=-not $Context.probe.HasExited} $Cleanup
            if($state.running) {
                Invoke-DeliveryCleanupStep 'probe-stdin-close' {$Context.probe.StandardInput.Close()} $Cleanup
                Invoke-DeliveryCleanupStep 'probe-grace-wait' {$state.exited=$Context.probe.WaitForExit(1000)} $Cleanup
                if(-not $state.exited) {
                    Invoke-DeliveryCleanupStep 'probe-kill' {$Context.probe.Kill()} $Cleanup
                    Invoke-DeliveryCleanupStep 'probe-final-wait' {
                        if(-not $Context.probe.WaitForExit(1000)){throw 'owned probe did not exit during cleanup'}
                    } $Cleanup
                }
            }
        } else { $Cleanup.stages.Add('probe-not-started') }
        Invoke-DeliveryCleanupStep 'probe-dispose' {$Context.probe.Dispose()} $Cleanup
    }
    if($null -ne $Tap) {
        Invoke-DeliveryCleanupStep 'watcher-dispose' {$Tap.Dispose()} $Cleanup
    }
    foreach($source in $SourceIds) {
        Invoke-DeliveryCleanupStep ('unregister:'+ $source) {
            Unregister-Event -SourceIdentifier $source -ErrorAction Stop
        } $Cleanup
        Invoke-DeliveryCleanupStep ('remove:'+ $source) {
            Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue |
                Remove-Event -ErrorAction Stop
        } $Cleanup
    }
}

function Publish-DeliveryControl {
    param([string]$Name,$Value)
    $destination=Join-Path $ControlDirectory $Name
    $temporary=$destination+'.tmp'
    $stream=[IO.File]::Open($temporary,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try {
        $bytes=[Text.UTF8Encoding]::new($false).GetBytes(($Value|ConvertTo-Json -Depth 10 -Compress))
        $stream.Write($bytes,0,$bytes.Length)
    } finally { $stream.Dispose() }
    [IO.File]::Move($temporary,$destination)
}

function Publish-DeliveryFinal {
    param($Result)
    try {
        Publish-DeliveryControl 'external-final.json' $Result
        return $true
    } catch {
        $message='final diagnostic publication: '+$_.Exception.Message
        $Result.publication_errors += [pscustomobject]@{stage='external-final.json';error=$message}
        if($null -eq $Result.error){$Result.error=$message}
        return $false
    }
}

function Read-DeliveryControl {
    param([string]$Name)
    $path=Join-Path $ControlDirectory $Name
    if([IO.File]::Exists($path)){return ([IO.File]::ReadAllText($path)|ConvertFrom-Json)}
    return $null
}

function Start-DeliveryProbe {
    param($Context,[switch]$FailStart)
    $info=[Diagnostics.ProcessStartInfo]::new()
    $info.FileName=Join-Path $env:SystemRoot 'System32/cmd.exe'
    if($FailStart){$info.FileName=Join-Path $env:TEMP ('cdr-nonexistent-'+[Guid]::NewGuid().ToString('N')+'.exe')}
    $info.Arguments='/d /q /c "echo CDR_DIAGNOSTIC_READY&set /p CDR_DIAG_RELEASE=&exit /b 0"'
    $info.UseShellExecute=$false; $info.CreateNoWindow=$true
    $info.RedirectStandardInput=$true; $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
    $Context.probe=[Diagnostics.Process]::new(); $Context.probe.StartInfo=$info
    if(-not $Context.probe.Start()){throw 'diagnostic probe did not start'}
    $Context.started=$true
    $Context.identity=[ordered]@{
        pid=$Context.probe.Id;parent_pid=$PID;name=$Context.probe.ProcessName+'.exe';
        created_tick=[string]$Context.probe.StartTime.ToUniversalTime().ToFileTimeUtc();
        ready=$false;ready_observed_tick=$null;ready_observed_utc_tick=$null;
        released=$false;release_tick=$null;release_utc_tick=$null;
        exited_tick=$null;exit_code=$null
    }
    $Context.ready_task=$Context.probe.StandardOutput.ReadLineAsync()
}

function Update-DeliveryReady {
    param($Context)
    if(-not $Context.identity.ready -and $Context.ready_task.IsCompleted) {
        $line=$Context.ready_task.GetAwaiter().GetResult()
        if($line -ne 'CDR_DIAGNOSTIC_READY'){throw 'unexpected diagnostic ready signal'}
        $Context.identity.ready=$true
        $Context.identity.ready_observed_tick=[string][Diagnostics.Stopwatch]::GetTimestamp()
        $Context.identity.ready_observed_utc_tick=[string][DateTime]::UtcNow.ToFileTimeUtc()
    }
}

function Release-DeliveryProbe {
    param($Context)
    if(-not $Context.identity.released) {
        $Context.probe.StandardInput.WriteLine('release')
        $Context.probe.StandardInput.Close()
        $Context.identity.released=$true
        $Context.identity.release_tick=[string][Diagnostics.Stopwatch]::GetTimestamp()
        $Context.identity.release_utc_tick=[string][DateTime]::UtcNow.ToFileTimeUtc()
    }
}

function Update-DeliveryExit {
    param($Context)
    if($Context.probe.HasExited) {
        $Context.identity.exited_tick=[string]$Context.probe.ExitTime.ToUniversalTime().ToFileTimeUtc()
        $Context.identity.exit_code=$Context.probe.ExitCode
    }
}

function Get-DeliveryCounts {
    param([object[]]$QueueRows,[object[]]$DirectRows,[long]$ProcessId,[string]$Label)
    $queue=@($QueueRows|Where-Object ProcessId -eq $ProcessId)
    $direct=@($DirectRows|Where-Object ProcessId -eq $ProcessId)
    return [pscustomobject]@{label=$Label;pid=$ProcessId;
        queue_start=@($queue|Where-Object Kind -eq 'start').Count;
        queue_stop=@($queue|Where-Object Kind -eq 'stop').Count;
        direct_start=@($direct|Where-Object Kind -eq 'start').Count;
        direct_stop=@($direct|Where-Object Kind -eq 'stop').Count}
}

$context=@{probe=$null;started=$false;identity=$null;ready_task=$null}
$tap=$null
$sourceIds=[Collections.Generic.List[string]]::new()
$cleanup=@{stages=[Collections.Generic.List[string]]::new();errors=[Collections.Generic.List[object]]::new()}
$script:cleanupTestTrace=[Collections.Generic.List[string]]::new()
$script:cleanupTestFault=$Fault
$clock=$null; $diagnosticExit=0; $releasePublished=$false
$hostProcess=[Diagnostics.Process]::GetCurrentProcess()
$result=[ordered]@{
    diagnostic_only=$true;payload_retried=$false;role=$Role;test_case=$null;
    host=[ordered]@{version=[string]$PSVersionTable.PSVersion;bits=([IntPtr]::Size*8);
        executable=$hostProcess.MainModule.FileName;root_pid=$PID;session_id=$hostProcess.SessionId;
        launch_parent_pid=$env:CDR_DIAGNOSTIC_PARENT_PID;
        admin=([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
            [Security.Principal.WindowsBuiltInRole]::Administrator)};
    acquisition_budget_ms=5000;acquisition_elapsed_ms=$null;launcher_ready=$null;
    signal=$null;subscriptions=[Collections.Generic.List[object]]::new();
    probes=[ordered]@{observer=$null;external=$null};release_reason=$null;comparison_valid=$false;
    polls=[Collections.Generic.List[object]]::new();queue_rows=@();direct_rows=@();
    correlation='PID-filtered diagnostic rows, not an ownership attestation';
    callback_errors=@();cleanup_errors=@();publication_errors=@();cleanup_stages=@();test_trace=@();error=$null;post_cleanup_json=$false
}
$hostProcess.Dispose()
try {
    if($Role -eq 'cleanup-test') {
        $result.test_case=$Fault
        $sourceIds.Add('cleanup-test-start'); $sourceIds.Add('cleanup-test-stop')
        function script:Unregister-Event {
            [CmdletBinding()]param([string]$SourceIdentifier)
            $script:cleanupTestTrace.Add('unregister:'+ $SourceIdentifier)
        }
        function script:Get-Event {
            [CmdletBinding()]param([string]$SourceIdentifier)
            $script:cleanupTestTrace.Add('remove:'+ $SourceIdentifier)
        }
        $tap=[pscustomobject]@{}
        $tap | Add-Member ScriptMethod Dispose {
            $script:cleanupTestTrace.Add('watcher-dispose')
            if($script:cleanupTestFault -eq 'watcher-cleanup'){throw 'injected watcher disposal failure'}
        }
        if($Fault -in @('publication-failure','publication-only')) {
            function script:Publish-DeliveryControl {
                param([string]$Name,$Value)
                if($Name -ne 'external-final.json'){throw 'unexpected diagnostic publication target'}
                throw 'injected final publication I/O failure'
            }
            if($Fault -eq 'publication-failure'){throw 'injected coordination timeout'}
        } elseif($Fault -eq 'start-failure') { Start-DeliveryProbe $context -FailStart }
        else {
            $context.probe=[pscustomobject]@{HasExited=($Fault -eq 'watcher-cleanup');StandardInput=[pscustomobject]@{}}
            $context.started=$true
            $context.probe.StandardInput | Add-Member ScriptMethod Close {}
            $context.probe | Add-Member ScriptMethod WaitForExit {param($timeout) return $false}
            $context.probe | Add-Member ScriptMethod Kill {throw 'injected kill failure'}
            $context.probe | Add-Member ScriptMethod Dispose {$script:cleanupTestTrace.Add('probe-dispose')}
            throw 'injected primary failure'
        }
    } else {
        if(-not [IO.Directory]::Exists($ControlDirectory)){throw 'owned diagnostic control directory is missing'}
        if($Role -eq 'launcher') {
            # This host is a sibling of the observer, launched by the Rust test parent.
            $guard=[Diagnostics.Stopwatch]::StartNew()
            Publish-DeliveryControl 'launcher-ready.json' ([ordered]@{host=$result.host;ready_tick=[string][Diagnostics.Stopwatch]::GetTimestamp()})
            while(-not [IO.File]::Exists((Join-Path $ControlDirectory 'start.json'))) {
                if([IO.File]::Exists((Join-Path $ControlDirectory 'abort'))){throw 'diagnostic controller canceled before launch'}
                if($guard.Elapsed.TotalSeconds -ge 20){throw 'diagnostic launch coordination timed out'}
                Start-Sleep -Milliseconds 10
            }
            $result.signal=Read-DeliveryControl 'start.json'
            if([IO.File]::Exists((Join-Path $ControlDirectory 'abort'))){throw 'diagnostic controller canceled before probe'}
            Start-DeliveryProbe $context
            $result.probes.external=$context.identity
            Publish-DeliveryControl 'external-created.json' $context.identity
            $readyPublished=$false
            while($guard.Elapsed.TotalSeconds -lt 20) {
                if([IO.File]::Exists((Join-Path $ControlDirectory 'abort'))){throw 'diagnostic controller canceled'}
                Update-DeliveryReady $context
                if($context.identity.ready -and -not $readyPublished) {
                    Publish-DeliveryControl 'external-ready.json' $context.identity
                    $readyPublished=$true
                }
                if($context.identity.ready -and [IO.File]::Exists((Join-Path $ControlDirectory 'release.json'))) {
                    Release-DeliveryProbe $context
                }
                Update-DeliveryExit $context
                if($null -ne $context.identity.exited_tick){break}
                Start-Sleep -Milliseconds 10
            }
            if($null -eq $context.identity.exited_tick){throw 'diagnostic probe coordination timed out'}
        } else {
            $result.launcher_ready=Read-DeliveryControl 'launcher-ready.json'
            if($null -eq $result.launcher_ready){throw 'external launcher was not ready before registration'}
Add-Type -AssemblyName System.Management
Add-Type -ReferencedAssemblies System.Management.dll, System.dll, System.Core.dll -TypeDefinition @'
using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.Management;
public sealed class CdrDeliveryRow {
    public string Kind;
    public uint ProcessId;
    public uint ParentProcessId;
    public string Name;
    public string CreatedTick;
    public string ReceivedTick;
}
public sealed class CdrDeliveryTap : IDisposable {
    public readonly ConcurrentQueue<CdrDeliveryRow> Rows = new ConcurrentQueue<CdrDeliveryRow>();
    public readonly ConcurrentQueue<string> Faults = new ConcurrentQueue<string>();
    public readonly ConcurrentQueue<string> CleanupFaults = new ConcurrentQueue<string>();
    private readonly ManagementEventWatcher start;
    private readonly ManagementEventWatcher stop;
    public CdrDeliveryTap() {
        start = Create("start", "Win32_ProcessStartTrace");
        stop = Create("stop", "Win32_ProcessStopTrace");
    }
    private ManagementEventWatcher Create(string kind, string className) {
        var watcher = new ManagementEventWatcher(
            new ManagementScope(@"\\.\root\cimv2"),
            new WqlEventQuery("SELECT * FROM " + className),
            new EventWatcherOptions());
        watcher.EventArrived += delegate(object sender, EventArrivedEventArgs args) {
            try {
                var value = args.NewEvent;
                Rows.Enqueue(new CdrDeliveryRow {
                    Kind = kind,
                    ProcessId = Convert.ToUInt32(value["ProcessID"], CultureInfo.InvariantCulture),
                    ParentProcessId = Convert.ToUInt32(value["ParentProcessID"], CultureInfo.InvariantCulture),
                    Name = Convert.ToString(value["ProcessName"], CultureInfo.InvariantCulture),
                    CreatedTick = Convert.ToString(value["TIME_CREATED"], CultureInfo.InvariantCulture),
                    ReceivedTick = Stopwatch.GetTimestamp().ToString(CultureInfo.InvariantCulture)
                });
            } catch (Exception error) { Faults.Enqueue(error.GetType().FullName + ": " + error.Message); }
        };
        return watcher;
    }
    public void Start() { start.Start(); stop.Start(); }
    private void Cleanup(string stage, Action action) {
        try { action(); }
        catch (Exception error) { CleanupFaults.Enqueue(stage + ": " + error.GetType().FullName + ": " + error.Message); }
    }
    public void Dispose() {
        Cleanup("start.Stop", delegate { start.Stop(); });
        Cleanup("stop.Stop", delegate { stop.Stop(); });
        Cleanup("start.Dispose", delegate { start.Dispose(); });
        Cleanup("stop.Dispose", delegate { stop.Dispose(); });
    }
}
'@

            $prefix='CdrDiagnostic'+[Guid]::NewGuid().ToString('N')
            $startId=$prefix+'Start'; $stopId=$prefix+'Stop'
            $clock=[Diagnostics.Stopwatch]::StartNew()
            foreach($entry in @(@{kind='start';class='Win32_ProcessStartTrace';source=$startId},
                                @{kind='stop';class='Win32_ProcessStopTrace';source=$stopId})) {
                $sourceIds.Add($entry.source)
                Register-WmiEvent -Namespace root\cimv2 -Class $entry.class -SourceIdentifier $entry.source | Out-Null
                $result.subscriptions.Add([pscustomobject]@{route='powershell';kind=$entry.kind;
                    namespace='root\cimv2';registered=$true;elapsed_ms=$clock.Elapsed.TotalMilliseconds})
            }
            $tap=[CdrDeliveryTap]::new(); $tap.Start()
            $result.subscriptions.Add([pscustomobject]@{route='direct';kind='start+stop';
                namespace='root\cimv2';registered=$true;elapsed_ms=$clock.Elapsed.TotalMilliseconds})
            if($clock.Elapsed.TotalMilliseconds -ge 5000){throw 'diagnostic registration consumed the acquisition budget'}
            $result.signal=[ordered]@{observer_pid=$PID;tick=[string][Diagnostics.Stopwatch]::GetTimestamp();
                utc_tick=[string][DateTime]::UtcNow.ToFileTimeUtc();elapsed_ms=$clock.Elapsed.TotalMilliseconds}
            Publish-DeliveryControl 'start.json' $result.signal
            Start-DeliveryProbe $context
            $result.probes.observer=$context.identity
            $externalCreated=$false; $externalReady=$false; $externalFinal=$false
            $queueRows=@(); $directRows=@()
            while($clock.Elapsed.TotalMilliseconds -lt 5000) {
                if([IO.File]::Exists((Join-Path $ControlDirectory 'abort'))){throw 'diagnostic controller canceled'}
                Update-DeliveryReady $context
                if(-not $externalCreated) {
                    $value=Read-DeliveryControl 'external-created.json'
                    if($null -ne $value){$result.probes.external=$value;$externalCreated=$true}
                }
                if($externalCreated -and -not $externalReady) {
                    $value=Read-DeliveryControl 'external-ready.json'
                    if($null -ne $value){$result.probes.external=$value;$externalReady=$true}
                }
                if($externalReady -and -not $externalFinal) {
                    $value=Read-DeliveryControl 'external-final.json'
                    if($null -ne $value){$result.probes.external=$value.probes.external;$externalFinal=$true}
                }
                $events=@(Get-Event -SourceIdentifier $startId -ErrorAction SilentlyContinue)+
                    @(Get-Event -SourceIdentifier $stopId -ErrorAction SilentlyContinue)
                $directRows=@($tap.Rows.ToArray())
                if($events.Count -gt 4096 -or $directRows.Count -gt 4096){throw 'diagnostic event limit exceeded'}
                $queueRows=@($events|ForEach-Object {
                    $eventValue=$_.SourceEventArgs.NewEvent
                    [pscustomobject]@{Kind=$(if($_.SourceIdentifier -eq $startId){'start'}else{'stop'});
                        ProcessId=[uint32]$eventValue.ProcessID;ParentProcessId=[uint32]$eventValue.ParentProcessID;
                        Name=[string]$eventValue.ProcessName;CreatedTick=[string]$eventValue.TIME_CREATED}
                })
                $externalId=if($externalCreated){[long]$result.probes.external.pid}else{0}
                $counts=@(
                    (Get-DeliveryCounts $queueRows $directRows $context.identity.pid 'observer')
                    (Get-DeliveryCounts $queueRows $directRows $externalId 'external')
                )
                $result.polls.Add([pscustomobject]@{elapsed_ms=$clock.Elapsed.TotalMilliseconds;counts=$counts;
                    queue_total=$queueRows.Count;direct_total=$directRows.Count})
                $allStarts=@($counts|Where-Object {$_.queue_start -eq 0 -or $_.direct_start -eq 0}).Count -eq 0
                if(-not $releasePublished -and $context.identity.ready -and $externalReady -and
                    ($allStarts -or $clock.Elapsed.TotalMilliseconds -ge 2500)) {
                    $result.release_reason=if($allStarts){'both-probes-both-start-routes'}else{'fixed-start-phase-deadline'}
                    Publish-DeliveryControl 'release.json' @{reason=$result.release_reason;tick=[string][Diagnostics.Stopwatch]::GetTimestamp()}
                    $releasePublished=$true
                    Release-DeliveryProbe $context
                }
                Update-DeliveryExit $context
                $allStops=@($counts|Where-Object {$_.queue_stop -eq 0 -or $_.direct_stop -eq 0}).Count -eq 0
                if($releasePublished -and $externalFinal -and $null -ne $context.identity.exited_tick -and $allStops){break}
                $left=5000-$clock.Elapsed.TotalMilliseconds
                if($left -gt 0){Start-Sleep -Milliseconds ([int][Math]::Min(25,[Math]::Ceiling($left)))}
            }
            $result.acquisition_elapsed_ms=$clock.Elapsed.TotalMilliseconds
            $ids=@($context.identity.pid)
            if($externalCreated){$ids+= $result.probes.external.pid}
            $result.queue_rows=@($queueRows|Where-Object {$_.ProcessId -in $ids})
            $result.direct_rows=@($directRows|Where-Object {$_.ProcessId -in $ids})
            $result.callback_errors=@($tap.Faults.ToArray())
            $result.comparison_valid=$externalReady -and $context.identity.ready -and $releasePublished -and
                ($result.probes.external.parent_pid -ne $PID) -and ($context.identity.parent_pid -eq $PID)
        }
    }
} catch {
    $result.error=$_.Exception.Message; $diagnosticExit=1
} finally {
    # Every stage is independent. Teardown is not observation evidence.
    if($Role -eq 'observer' -and -not $releasePublished -and [IO.Directory]::Exists($ControlDirectory)) {
        Invoke-DeliveryCleanupStep 'release-external' {
            Publish-DeliveryControl 'release.json' @{reason='observer-cleanup';tick=[string][Diagnostics.Stopwatch]::GetTimestamp()}
        } $cleanup
    }
    Complete-DeliveryCleanup $context $tap $sourceIds.ToArray() $cleanup
    if($Role -eq 'observer' -and $null -ne $tap) {
        foreach($faultMessage in $tap.CleanupFaults.ToArray()) {
            $cleanup.errors.Add([pscustomobject]@{stage='direct-watcher';error=$faultMessage})
        }
    }
    $result.cleanup_errors=@($cleanup.errors.ToArray())
    $result.cleanup_stages=@($cleanup.stages.ToArray())
    $result.test_trace=@($script:cleanupTestTrace.ToArray())
    if($cleanup.errors.Count -gt 0){$diagnosticExit=1}
    $result.post_cleanup_json=$true
    if($Role -eq 'launcher' -or
        ($Role -eq 'cleanup-test' -and $Fault -in @('publication-failure','publication-only'))) {
        if(-not (Publish-DeliveryFinal $result)){$diagnosticExit=1}
    }
    [Console]::Out.WriteLine('EVENT_DELIVERY_DIAGNOSTIC '+($result|ConvertTo-Json -Depth 12 -Compress))
}
exit $diagnosticExit
