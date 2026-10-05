# Diagnostic only. No native payload, no readiness override, no privilege changes.
param([ValidateSet('capture', 'contracts', 'late-arrival', 'kernel-compare', 'kernel-contracts', 'kernel-regressions')][string]$Mode = 'kernel-compare')
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Set-StrictMode -Version Latest
if ($Mode -in @('kernel-compare', 'kernel-contracts', 'kernel-regressions')) {
    $kernelMode = if ($Mode -eq 'kernel-contracts') {'contracts'} elseif ($Mode -eq 'kernel-regressions') {'regressions'} else {'capture'}
    & (Join-Path $PSScriptRoot 'native_process_kernel_compare.ps1') -Mode $kernelMode
    exit $LASTEXITCODE
}
. (Join-Path $PSScriptRoot 'native_process_creation_matrix.ps1') -FunctionsOnly
Add-Type -Path @((Join-Path $PSScriptRoot 'native_process_route_matrix.cs'), (Join-Path $PSScriptRoot 'native_process_semisync.cs')) -ReferencedAssemblies System.Management.dll, System.dll, System.Core.dll

function Complete-SemisyncObservers {
    param($Sync, $Direct, $Failures)
    foreach ($entry in @(@{name='semisync-dispose'; value=$Sync}, @{name='direct-dispose'; value=$Direct})) {
        try { $entry.value.Dispose() } catch { $Failures.Add(@{stage=$entry.name; error=$_.Exception.Message}) }
    }
}

function Invoke-SemisyncCleanupContract {
    param([string]$Case)
    $tap=[CdrSemisyncTap]::ForCleanupContract($Case)
    $direct=[pscustomobject]@{disposed=$false}
    Add-Member -InputObject $direct -MemberType ScriptMethod -Name Dispose -Value {$this.disposed=$true}
    $failures=[Collections.Generic.List[object]]::new()
    $primary='injected primary collection failure'
    try { $tap.Start() } catch { $primary=$_.Exception.Message }
    finally { Complete-SemisyncObservers $tap $direct $failures }
    $expectedJoined=$Case -eq 'partial_start'
    $reaped=$tap.ReapContractThreads()
    $expectedPrimary=if ($expectedJoined) {'injected second thread start failure'} else {'injected primary collection failure'}
    $pass=$reaped -and $direct.disposed -and $failures.Count -eq 0 -and
        $primary.Contains($expectedPrimary) -and $tap.ThreadsJoined -eq $expectedJoined -and
        $tap.Started[0] -and $tap.Started[1] -eq (-not $expectedJoined) -and
        $tap.JoinAttempted[0] -and $tap.JoinAttempted[1] -eq (-not $expectedJoined) -and
        $tap.CleanupFaults.IsEmpty -eq $expectedJoined
    return @{name=($Case+'_cleanup'); pass=$pass; primary_error=$primary; threads_started=$tap.Started
        join_attempted=$tap.JoinAttempted; threads_joined=$tap.ThreadsJoined; contract_threads_reaped=$reaped
        direct_disposed=$direct.disposed; cleanup_errors=$tap.CleanupFaults.ToArray(); synthetic=$true}
}

function Invoke-SemisyncOuterCleanupContract {
    param([bool]$Both)
    $failures=[Collections.Generic.List[object]]::new()
    $sync=[pscustomobject]@{disposed=$false}
    Add-Member -InputObject $sync -MemberType ScriptMethod -Name Dispose -Value {$this.disposed=$true;throw 'injected sync dispose failure'}
    $direct=[pscustomobject]@{disposed=$false; fail=$Both}
    Add-Member -InputObject $direct -MemberType ScriptMethod -Name Dispose -Value {$this.disposed=$true;if($this.fail){throw 'injected direct dispose failure'}}
    $primary='injected primary remains'
    Complete-SemisyncObservers $sync $direct $failures
    $expected=if ($Both) {2} else {1}
    $name=if ($Both) {'outer_both_dispose_failures'} else {'outer_sync_dispose_failure'}
    return @{name=$name; pass=($sync.disposed -and $direct.disposed -and $failures.Count -eq $expected -and $primary -ceq 'injected primary remains')
        primary_error=$primary; cleanup_errors=$failures.ToArray(); after_cleanups_reached=$true; synthetic=$true}
}

function Invoke-SemisyncDeadlineContracts {
    foreach ($case in @(
        @{name='ready_before_deadline'; ready=$true; ms=999; expected=$true},
        @{name='ready_at_deadline'; ready=$true; ms=1000; expected=$false},
        @{name='ready_after_deadline'; ready=$true; ms=1001; expected=$false},
        @{name='not_ready_before_deadline'; ready=$false; ms=999; expected=$false},
        @{name='negative_clock_rejected'; ready=$true; ms=-1; expected=$false}
    )) {
        $gate=[CdrSemisyncGate]::new();$actual=$gate.Authorize($case.ready,$case.ms)
        @{name=$case.name; pass=($actual -eq $case.expected); elapsed_ms=$case.ms; authorized=$actual}
    }
    $gate=[CdrSemisyncGate]::new();$first=$gate.Authorize($true,999);$second=$gate.Authorize($true,999)
    @{name='authorization_is_one_shot'; pass=($first -and -not $second)}
}

if ($Mode -eq 'contracts') {
    $cases = @(
        @{name='only_poll_timeout_is_retriable'; pass=[CdrSemisyncTap]::IsPollTimeout([int][Management.ManagementStatus]::Timedout)},
        @{name='access_denied_is_not_timeout'; pass=(-not [CdrSemisyncTap]::IsPollTimeout([int][Management.ManagementStatus]::AccessDenied))},
        @{name='transport_failure_is_not_timeout'; pass=(-not [CdrSemisyncTap]::IsPollTimeout([int][Management.ManagementStatus]::Failed))},
        @{name='success_is_not_timeout'; pass=(-not [CdrSemisyncTap]::IsPollTimeout(0))}
    )
    foreach ($case in @('partial_start','join_throw','join_false')) { $cases += Invoke-SemisyncCleanupContract $case }
    $cases += Invoke-SemisyncOuterCleanupContract $false
    $cases += Invoke-SemisyncOuterCleanupContract $true
    $cases += @(Invoke-SemisyncDeadlineContracts)
    @{diagnostic_only=$true; native_gate_pass=$false; contracts=$cases} | ConvertTo-Json -Depth 5 -Compress
    if (@($cases | Where-Object {-not $_.pass}).Count -gt 0) {exit 1}
    exit 0
}
$clock = [Diagnostics.Stopwatch]::StartNew()
$windowStartedQpc = [Diagnostics.Stopwatch]::GetTimestamp().ToString()
$windowEndedQpc = $null
$begin = [DateTime]::UtcNow.ToFileTimeUtc().ToString()
$prefix = 'cdr-semisync-' + [Guid]::NewGuid().ToString('N')
$subscriptions = [Collections.Generic.List[string]]::new()
$queueRows = [Collections.Generic.List[object]]::new()
$probes = [Collections.Generic.List[object]]::new()
$cleanup = [Collections.Generic.List[object]]::new()
$direct = [CdrRouteTap]::new($clock)
$sync = [CdrSemisyncTap]::new($clock)
$gate = [CdrSemisyncGate]::new()
$primary = $null; $completed = $false; $syncReady = $false; $end = $null
$context = [CdrRouteContext]::Observer()
$threadContext = [CdrRouteContext]::Thread()
try {
    foreach ($kind in @('start','stop')) {
        $class = if ($kind -eq 'start') {'Win32_ProcessStartTrace'} else {'Win32_ProcessStopTrace'}
        $id = $prefix + '-' + $kind
        $null = Register-WmiEvent -Class $class -SourceIdentifier $id -ErrorAction Stop
        $subscriptions.Add($id)
    }
    $direct.Start(); $sync.Start()
    while (-not $sync.Ready -and $clock.ElapsedMilliseconds -lt 1000 -and $sync.Faults.IsEmpty) {
        Start-Sleep -Milliseconds 10
    }
    $syncReady = $gate.Authorize($sync.Ready, $clock.ElapsedMilliseconds)
    if (-not $syncReady) { throw 'Semisynchronous subscription did not complete an enumeration within 1s; probes were not launched' }
    foreach ($held in @($false,$true)) {
        $process = [Diagnostics.Process]::new()
        $probe = [pscustomobject]@{
            process=$process; held=$held; started=$false; released=$false; process_id=0; creator_pid=$PID
            start_before_qpc=$null; start_after_qpc=$null; created_filetime_utc=$null; exit_filetime_utc=$null; exit_confirmed=$false; exit_code=$null
            termination_requested=$false; ready_task=$null; ready_checked=$false; ready=$false; ready_ms=$null; release_ms=$null
        }
        $probes.Add($probe)
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = Join-Path $env:SystemRoot 'System32\cmd.exe'
        $info.Arguments = if ($held) {'/d /q /c "echo CDR_SYNC_READY&set /p CDR_SYNC_RELEASE=&exit /b 0"'} else {'/d /q /c "echo CDR_SYNC_READY&exit /b 0"'}
        $info.UseShellExecute=$false; $info.CreateNoWindow=$true
        $info.RedirectStandardInput=$true; $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
        $process.StartInfo=$info
        $probe.start_before_qpc=[Diagnostics.Stopwatch]::GetTimestamp().ToString()
        if (-not $process.Start()) { throw 'Diagnostic probe did not start' }
        $probe.start_after_qpc=[Diagnostics.Stopwatch]::GetTimestamp().ToString()
        $probe.started=$true; $probe.process_id=$process.Id
        $probe.created_filetime_utc=$process.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()
        $probe.ready_task=$process.StandardOutput.ReadLineAsync()
        if (-not $held) { $process.StandardInput.Close(); $probe.released=$true; $probe.release_ms=$clock.ElapsedMilliseconds }
    }
    while ($clock.ElapsedMilliseconds -lt 5000) {
        foreach ($event in @(Get-Event | Where-Object { $_.SourceIdentifier -in $subscriptions })) {
            if ($queueRows.Count -ge 4096) { throw 'PowerShell raw event row limit exceeded' }
            $value=$event.SourceEventArgs.NewEvent
            $queueRows.Add([pscustomobject]@{
                Kind=$(if ($event.SourceIdentifier.EndsWith('-start')) {'start'} else {'stop'})
                ProcessId=[uint32]$value.ProcessID; ParentProcessId=[uint32]$value.ParentProcessID
                Name=[string]$value.ProcessName; CreatedTick=[string]$value.TIME_CREATED
                ReceivedUtcTick=[DateTime]::UtcNow.ToFileTimeUtc().ToString(); ReceivedMs=$clock.ElapsedMilliseconds
            })
            Remove-Event -EventIdentifier $event.EventIdentifier -ErrorAction Stop
        }
        foreach ($probe in $probes) {
            if (-not $probe.ready_checked -and $probe.ready_task.IsCompleted) {
                $probe.ready_checked=$true; $probe.ready=($probe.ready_task.GetAwaiter().GetResult() -ceq 'CDR_SYNC_READY')
                $probe.ready_ms=$clock.ElapsedMilliseconds
            }
            if ($probe.held -and $probe.ready -and -not $probe.released -and $clock.ElapsedMilliseconds -ge 2500) {
                $probe.process.StandardInput.WriteLine(''); $probe.process.StandardInput.Close()
                $probe.released=$true; $probe.release_ms=$clock.ElapsedMilliseconds
            }
        }
        Start-Sleep -Milliseconds 10
    }
    $windowEndedQpc=[Diagnostics.Stopwatch]::GetTimestamp().ToString()
    $end=[DateTime]::UtcNow.ToFileTimeUtc().ToString()
    $completed=$true
    # Diagnostic tail only: no new probe or requested command is started here.
    # The original acquisition verdict remains limited to the first 5000 ms.
    if ($Mode -eq 'late-arrival') {
        while ($clock.ElapsedMilliseconds -lt 20000) {
            foreach ($event in @(Get-Event | Where-Object { $_.SourceIdentifier -in $subscriptions })) {
                if ($queueRows.Count -ge 4096) { throw 'PowerShell raw event row limit exceeded' }
                $value=$event.SourceEventArgs.NewEvent
                $queueRows.Add([pscustomobject]@{
                    Kind=$(if ($event.SourceIdentifier.EndsWith('-start')) {'start'} else {'stop'})
                    ProcessId=[uint32]$value.ProcessID; ParentProcessId=[uint32]$value.ParentProcessID
                    Name=[string]$value.ProcessName; CreatedTick=[string]$value.TIME_CREATED
                    ReceivedUtcTick=[DateTime]::UtcNow.ToFileTimeUtc().ToString(); ReceivedMs=$clock.ElapsedMilliseconds
                })
                Remove-Event -EventIdentifier $event.EventIdentifier -ErrorAction Stop
            }
            Start-Sleep -Milliseconds 10
        }
    }
} catch { $primary=$_.Exception.Message }
finally {
    if ($null -eq $windowEndedQpc) { $windowEndedQpc=[Diagnostics.Stopwatch]::GetTimestamp().ToString() }
    if ($null -eq $end) { $end=[DateTime]::UtcNow.ToFileTimeUtc().ToString() }
    foreach ($probe in $probes) { Complete-MatrixProbe $probe $cleanup }
    Complete-SemisyncObservers $sync $direct $cleanup
    foreach ($id in $subscriptions) {
        try { Unregister-Event -SourceIdentifier $id -ErrorAction Stop } catch { $cleanup.Add(@{stage='unsubscribe';error=$_.Exception.Message}) }
        try {
            foreach ($event in @(Get-Event | Where-Object SourceIdentifier -eq $id)) { Remove-Event -EventIdentifier $event.EventIdentifier -ErrorAction Stop }
        } catch { $cleanup.Add(@{stage='remove-event';error=$_.Exception.Message}) }
    }
}
$publicProbes=@($probes | Select-Object held,started,process_id,creator_pid,start_before_qpc,start_after_qpc,created_filetime_utc,exit_filetime_utc,ready,ready_ms,released,release_ms,exit_confirmed,exit_code,termination_requested)
$safe=$sync.ThreadsJoined -and $cleanup.Count -eq 0 -and $sync.CleanupFaults.IsEmpty -and $direct.CleanupFaults.IsEmpty -and @($probes | Where-Object {$_.started -and (-not $_.exit_confirmed -or $_.termination_requested)}).Count -eq 0
$valid=$safe -and $completed -and $null -eq $primary -and $sync.Faults.IsEmpty -and $direct.Faults.IsEmpty -and $probes.Count -eq 2
foreach ($probe in $probes) {
    if (-not ($probe.ready -and $probe.released -and $probe.exit_confirmed -and $probe.exit_code -eq 0 -and [CdrRouteRules]::Lifetime($begin,$end,$probe.created_filetime_utc,$probe.exit_filetime_utc))) {$valid=$false}
}
[ordered]@{
    diagnostic_only=$true; native_gate_pass=$false; comparison_valid=$valid; safe_for_follow_up=$safe
    scope='One observer, same producer context; not the independent-context experiment'
    observer_pid=$PID; observer_process=$context; observer_thread=$threadContext; semisync_threads=$sync.ThreadContexts
    acquisition_budget_ms=5000; sync_enumeration_ready=$syncReady; sync_poll_timeouts=$sync.PollTimeouts
    first_enumeration_ms=$sync.FirstEnumerationMs; threads_started=$sync.Started; join_attempted=$sync.JoinAttempted; threads_joined=$sync.ThreadsJoined
    window_started_qpc=$windowStartedQpc; window_ended_qpc=$windowEndedQpc; qpc_frequency=[Diagnostics.Stopwatch]::Frequency
    window_started_filetime_utc=$begin; window_ended_filetime_utc=$end; elapsed_with_cleanup_ms=$clock.ElapsedMilliseconds
    probes=$publicProbes
    queue_rows=@($queueRows.ToArray() | Where-Object {$_.ReceivedMs -lt 5000})
    direct_rows=@($direct.Rows.ToArray() | Where-Object {$_.ReceivedMs -lt 5000})
    semisync_rows=@($sync.Rows.ToArray() | Where-Object {$_.ReceivedMs -lt 5000})
    late_observation_budget_ms=$(if ($Mode -eq 'late-arrival') {15000} else {0})
    late_queue_rows=@($queueRows.ToArray() | Where-Object {$_.ReceivedMs -ge 5000})
    late_direct_rows=@($direct.Rows.ToArray() | Where-Object {$_.ReceivedMs -ge 5000})
    late_semisync_rows=@($sync.Rows.ToArray() | Where-Object {$_.ReceivedMs -ge 5000})
    error=$primary; callback_errors=$direct.Faults.ToArray(); semisync_errors=$sync.Faults.ToArray()
    cleanup_errors=$cleanup.ToArray(); direct_cleanup_errors=$direct.CleanupFaults.ToArray(); semisync_cleanup_errors=$sync.CleanupFaults.ToArray()
} | ConvertTo-Json -Depth 9 -Compress
if ($valid) {exit 0} else {exit 1}
