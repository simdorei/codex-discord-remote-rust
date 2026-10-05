Import-Module (Join-Path $PSScriptRoot 'CdrNativeProcess.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

function Test-CdrCompatibleStopName([string]$StartedName, [string]$StoppedName) {
    if ([string]::IsNullOrWhiteSpace($StartedName) -or
        [string]::IsNullOrWhiteSpace($StoppedName)) { return $false }
    if ($StartedName.Equals($StoppedName, [StringComparison]::OrdinalIgnoreCase)) { return $true }
    # The exercised Windows stop trace truncates ASCII image names to 14 chars.
    # Do not treat arbitrary prefixes or unverified multibyte truncation as identity.
    return $StartedName -cmatch '^[\x20-\x7E]+$' -and $StartedName.Length -gt 14 -and
        $StoppedName.Length -eq 14 -and
        $StartedName.StartsWith($StoppedName, [StringComparison]::OrdinalIgnoreCase)
}

function Find-CdrObservedInstance([object[]]$Owned, [object]$Identity, [uint32]$RootProcessId) {
    if ($null -eq $Identity) { return $null }
    # ETW timestamps identify the event, not its later delivery to this observer.
    # Require one unique PID lifecycle with a start inside the actual OS lifetime.
    # A later same-PID process cannot stand in for a missing command start.
    $matches = @($Owned | Where-Object { $_.pid -eq $Identity.pid })
    if ($matches.Count -ne 1 -or $matches[0].parent -ne $RootProcessId -or
        $matches[0].name -ine $Identity.name -or $matches[0].start_tick -lt $Identity.created_tick -or
        $matches[0].start_tick -gt $Identity.exited_tick -or
        $matches[0].stop_tick -le $matches[0].start_tick) { return $null }
    return $matches[0]
}

function Get-CdrObservedProcessState {
    param([object[]]$Events, [string]$StartId, [string]$StopId, [uint32]$RootProcessId,
        [object[]]$Canaries, [object]$CommandProcess)
    $active = [Collections.Generic.Dictionary[uint32,object]]::new()
    $owned = [Collections.Generic.List[object]]::new()
    # Rebuild by event time on every drain, so a late-delivered stop is matched
    # before a later PID reuse. Keep every owned instance's stop obligation.
    foreach ($event in @($Events | Sort-Object { [uint64]$_.SourceEventArgs.NewEvent.TIME_CREATED })) {
        $row = $event.SourceEventArgs.NewEvent
        $eventPid = [uint32]$row.ProcessID
        $tick = [uint64]$row.TIME_CREATED
        if ($event.SourceIdentifier -eq $StopId) {
            # PID/order identifies the candidate, but contradictory stop metadata
            # cannot discharge its obligation when a foreign start is not collected.
            if ($active.ContainsKey($eventPid)) {
                $candidate = $active[$eventPid]
                $stopParent = [uint32]$row.ParentProcessID
                # Windows may report zero for an unknown stop-trace parent.
                if ($tick -gt $candidate.start_tick -and
                    ($stopParent -eq 0 -or $stopParent -eq $candidate.parent) -and
                    (Test-CdrCompatibleStopName $candidate.name ([string]$row.ProcessName))) {
                    $candidate.stop_tick = $tick
                    $null = $active.Remove($eventPid)
                }
            }
            continue
        }
        if ($event.SourceIdentifier -ne $StartId) { continue }
        $parent = [uint32]$row.ParentProcessID
        $isOwned = $parent -eq $RootProcessId -or ($active.ContainsKey($parent) -and $active[$parent].owned)
        $instance = [pscustomobject]@{
            name = [string]$row.ProcessName; pid = $eventPid; parent = $parent
            start_tick = $tick; stop_tick = [uint64]0; owned = $isOwned
        }
        $active[$eventPid] = $instance
        if ($isOwned) { $owned.Add($instance) }
    }
    $usedInstances = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $canaryComplete = $Canaries.Count -eq 2
    foreach ($identity in $Canaries) {
        $instance = Find-CdrObservedInstance $owned.ToArray() $identity $RootProcessId
        if ($identity.name -ine 'cmd.exe' -or $null -eq $instance -or
            -not $usedInstances.Add("$($instance.pid)@$($instance.start_tick)")) { $canaryComplete = $false }
    }
    $command = Find-CdrObservedInstance $owned.ToArray() $CommandProcess $RootProcessId
    if ($null -ne $command -and -not $usedInstances.Add("$($command.pid)@$($command.start_tick)")) { $command = $null }
    $remaining = @($owned | Where-Object { $_.stop_tick -eq 0 })
    return [pscustomobject]@{
        ready = $canaryComplete -and $null -ne $command -and $remaining.Count -eq 0
        canary_observed = $canaryComplete; owned = [object[]]$owned.ToArray()
        command_process_observed = $null -ne $command; command = $command
        remaining = @($remaining | ForEach-Object { "$($_.pid)@$($_.start_tick)" })
    }
}

function Start-CdrProcessObserver {
    param([string]$StartId, [string]$StopId)
    if ($null -eq ('CdrKernelTap' -as [type])) {
        $source = Join-Path $PSScriptRoot '../crates/cdr-runtime/tests/fixtures/native_process_kernel_trace.cs'
        Add-Type -Path $source -ErrorAction Stop
    }
    $api = [CdrKernelApi]::new(('CDR-QA-Observed-' + [guid]::NewGuid().ToString('N')), $false)
    $observer = [CdrKernelTap]::new($api)
    try { $observer.Start(); return $observer }
    catch {
        $original = $_
        $observer.Complete()
        if (-not $observer.Safe) {
            throw [InvalidOperationException]::new(($original.Exception.Message + '; ETW startup cleanup unconfirmed'), $original.Exception)
        }
        throw $original
    }
}

function Get-CdrProcessObserverEvents {
    param($Observer, [string]$StartId, [string]$StopId, [switch]$Final)
    if ($Observer.DecodeErrors -ne 0 -or $Observer.Dropped -ne 0 -or -not $Observer.Faults.IsEmpty) {
        throw 'Kernel process observation lost or could not decode events; no passing record was produced.'
    }
    if (-not $Final) {
        if ($Observer.ProcessStatus -ne -1) { throw 'Kernel process observer exited before completion.' }
        $Observer.Flush()
    }
    foreach ($row in $Observer.Rows.ToArray()) {
        $source = if ($row.Kind -eq 'start') { $StartId } else { $StopId }
        # This API instance disables RAW_TIMESTAMP: EventQpc contains UTC FILETIME.
        [pscustomobject]@{SourceIdentifier=$source;SourceEventArgs=[pscustomobject]@{NewEvent=[pscustomobject]@{
            TIME_CREATED=[uint64]$row.EventQpc;ProcessID=$row.ProcessId;
            ParentProcessID=$row.ParentProcessId;ProcessName=$row.Image}}}
    }
}

function Stop-CdrProcessObserver {
    param($Observer)
    $Observer.Complete()
    if (-not $Observer.Safe -or -not $Observer.CompleteCapture) {
        throw 'Kernel process observation capture or cleanup was incomplete; no passing record was produced.'
    }
}

function Wait-CdrProcessObserverReady {
    param([string]$StartId, [string]$StopId, [TimeSpan]$Budget, $Observer)
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $probes = [Collections.Generic.List[object]]::new()
    while ($clock.Elapsed -lt $Budget) {
        $events = @(Get-CdrProcessObserverEvents -Observer $Observer -StartId $StartId -StopId $StopId)
        if ($events.Count -gt 4096) { throw 'Process observer event limit exceeded; no passing record was produced.' }
        $state = Get-CdrObservedProcessState -Events $events -StartId $StartId -StopId $StopId `
            -RootProcessId $PID -Canaries @() -CommandProcess $null
        if ($state.remaining.Count -eq 0) {
            foreach ($probe in $probes) {
                if ($null -ne (Find-CdrObservedInstance $state.owned $probe $PID) -and
                    $clock.Elapsed -lt $Budget) {
                    return [pscustomobject]@{ canary = $probe; waited = $clock.Elapsed }
                }
            }
            # Only harmless readiness probes may repeat. The requested command
            # is not dispatched until an actual start/stop pair is observed.
            $probe = $null
            $null = Invoke-CdrNative -Executable (Join-Path $env:SystemRoot 'System32/cmd.exe') `
                -Arguments @('/d', '/c', 'exit', '0') -TimeoutSeconds 1 -ProcessIdentity ([ref]$probe)
            $probes.Add($probe)
        }
        # Keep unresolved owned starts; never replace or discard their stops.
        if ($clock.Elapsed -lt $Budget) { Start-Sleep -Milliseconds 50 }
    }
    throw 'Process observation readiness timed out; command was not started. No passing record was produced.'
}

# Bounded current-PC evidence, not proof that shared/renamed interpreters are absent.
# Call only with public-safe test arguments; never pass credentials on command lines.
function Invoke-CdrObservedNativeCommand {
    [CmdletBinding()]
    param([string]$Id, [string]$Executable, [string[]]$Arguments, [int]$TimeoutSeconds = 600, [switch]$OfflineFixture)
    $ErrorActionPreference = 'Stop'
    if (-not $OfflineFixture) { throw 'Only independently reviewed offline fixture commands may enter this observation record; live checks are separate.' }
    $startId = 'CdrObservedStart-' + [guid]::NewGuid().ToString('N')
    $stopId = 'CdrObservedStop-' + [guid]::NewGuid().ToString('N')
    $canaries = [Collections.Generic.List[object]]::new()
    $observationStart = [DateTime]::UtcNow
    $observer = $null; $stopped = $false; $failure = $null; $cleanupFailure = $null; $record = $null
    try {
        $observer = Start-CdrProcessObserver -StartId $startId -StopId $stopId
        $waitBudget = [TimeSpan]::FromSeconds(5)
        $ready = Wait-CdrProcessObserverReady -StartId $startId -StopId $stopId -Budget $waitBudget -Observer $observer
        $canaries.Add($ready.canary)
        $canary = $null
        $started = [DateTime]::UtcNow
        $commandProcess = $null
        $output = Invoke-CdrNative -Executable $Executable -Arguments $Arguments -TimeoutSeconds $TimeoutSeconds -ProcessIdentity ([ref]$commandProcess)
        $ended = [DateTime]::UtcNow
        $null = Invoke-CdrNative -Executable (Join-Path $env:SystemRoot 'System32/cmd.exe') `
            -Arguments @('/d', '/c', 'exit', '0') -ProcessIdentity ([ref]$canary)
        $canaries.Add($canary)
        # Readiness and final drain share the original five-second wait budget.
        $deadline = [DateTime]::UtcNow.Add($waitBudget - $ready.waited)
        do {
            $events = @(Get-CdrProcessObserverEvents -Observer $observer -StartId $startId -StopId $stopId)
            if ($events.Count -gt 4096) { throw 'Process observer event limit exceeded; no passing record was produced.' }
            $snapshot = Get-CdrObservedProcessState -Events $events -StartId $startId -StopId $stopId `
                -RootProcessId $PID -Canaries $canaries.ToArray() -CommandProcess $commandProcess
            if (-not $snapshot.ready) { Start-Sleep -Milliseconds 50 }
        } while (-not $snapshot.ready -and [DateTime]::UtcNow -lt $deadline)
        if (-not $snapshot.ready) {
            throw "Process observation incomplete: canaries=$($snapshot.canary_observed); command=$($snapshot.command_process_observed); pending owned instances=$($snapshot.remaining -join ','). No passing record was produced."
        }
        # Close only our session, then recheck the final loss-free capture before publishing.
        $stopped = $true
        Stop-CdrProcessObserver -Observer $observer
        $events = @(Get-CdrProcessObserverEvents -Observer $observer -StartId $startId -StopId $stopId -Final)
        $snapshot = Get-CdrObservedProcessState -Events $events -StartId $startId -StopId $stopId `
            -RootProcessId $PID -Canaries $canaries.ToArray() -CommandProcess $commandProcess
        if (-not $snapshot.ready) { throw 'Final kernel process observation is incomplete; no passing record was produced.' }
        $owned = $snapshot.owned
        $interpreters = @($owned | Where-Object { $_.name -match '^(?:py|pyw|python(?:w|\d[\d.]*)?|pypy[\d.]*)\.exe$' })
        if ($interpreters.Count -ne 0) { throw "Observed $($interpreters.Count) recognized Python starts; no passing record was produced." }
        $record = [pscustomobject][ordered]@{
            id = $Id; command = $Executable + ' ' + ($Arguments -join ' '); exit_code = 0; status = 'completed'
            observer = 'windows_process_start_stop_trace'; observer_backend = 'kernel_etw'; started_at = $started.ToString('o'); ended_at = $ended.ToString('o')
            observation_started_at = $observationStart.ToString('o'); observation_ended_at = [DateTime]::UtcNow.ToString('o')
            canary_observed = $true; owned_process_starts = $owned.Count; recognized_python_process_count = $interpreters.Count
            command_process = [pscustomobject]@{
                observed = $true; pid = $commandProcess.pid; name = $commandProcess.name
                created_at = [DateTime]::FromFileTimeUtc($commandProcess.created_tick).ToString('o')
                exited_at = [DateTime]::FromFileTimeUtc($commandProcess.exited_tick).ToString('o')
                start_event_at = [DateTime]::FromFileTimeUtc($snapshot.command.start_tick).ToString('o')
                stop_event_at = [DateTime]::FromFileTimeUtc($snapshot.command.stop_tick).ToString('o')
            }
            live_network_invoked = $false; stdout = [string]($output -join "`n")
        }
    } catch { $failure = $_ }
    finally {
        if ($null -ne $observer -and -not $stopped) {
            try { Stop-CdrProcessObserver -Observer $observer } catch { $cleanupFailure = $_ }
        }
    }
    if ($null -ne $failure) {
        if ($null -ne $cleanupFailure) {
            throw [InvalidOperationException]::new(($failure.Exception.Message + '; observer cleanup: ' + $cleanupFailure.Exception.Message), $failure.Exception)
        }
        throw $failure
    }
    if ($null -ne $cleanupFailure) { throw $cleanupFailure }
    return $record
}

Export-ModuleMember -Function 'Invoke-CdrObservedNativeCommand', 'Get-CdrObservedProcessState'
