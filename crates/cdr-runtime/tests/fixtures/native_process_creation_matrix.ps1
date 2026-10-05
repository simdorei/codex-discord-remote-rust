param(
    [ValidateSet('none', 'close-error', 'initial-wait-error', 'kill-error', 'final-wait-false')]
    [string]$CleanupTestCase = 'none',
    [switch]$FunctionsOnly
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Set-StrictMode -Version Latest

function Add-MatrixCleanupFailure {
    param($Failures, [string]$Stage, [int]$ProcessId, [string]$Message)
    $Failures.Add([pscustomobject]@{
        stage = $Stage; process_id = $ProcessId; error = $Message
    })
}

function Complete-MatrixProbe {
    param($Probe, $Failures)
    if ($Probe.started) {
        if (-not $Probe.released) {
            try { $Probe.process.StandardInput.Close() } catch {
                Add-MatrixCleanupFailure $Failures 'probe-stdin-close' $Probe.process_id $_.Exception.Message
            }
        }
        $exitedInitially = $false
        try { $exitedInitially = $Probe.process.WaitForExit(1000) } catch {
            Add-MatrixCleanupFailure $Failures 'probe-wait-initial' $Probe.process_id $_.Exception.Message
        }
        if (-not $exitedInitially) {
            $Probe.termination_requested = $true
            try { $Probe.process.Kill() } catch {
                Add-MatrixCleanupFailure $Failures 'probe-kill' $Probe.process_id $_.Exception.Message
            }
        }
        try {
            if (-not $Probe.process.WaitForExit(1000)) { throw 'Probe exit remains unconfirmed' }
            $Probe.exit_confirmed = $true
        } catch {
            Add-MatrixCleanupFailure $Failures 'probe-exit-confirm' $Probe.process_id $_.Exception.Message
        }
        if ($Probe.exit_confirmed) {
            try {
                $Probe.exit_code = $Probe.process.ExitCode
                $Probe.exit_filetime_utc = $Probe.process.ExitTime.ToUniversalTime().ToFileTimeUtc().ToString()
            } catch {
                Add-MatrixCleanupFailure $Failures 'probe-exit-identity' $Probe.process_id $_.Exception.Message
            }
        }
    }
    try { $Probe.process.Dispose() } catch {
        Add-MatrixCleanupFailure $Failures 'probe-dispose' $Probe.process_id $_.Exception.Message
    }
}

function Invoke-MatrixCleanupFault {
    param([string]$Case)
    $trace = [Collections.Generic.List[string]]::new()
    $inputStream = [pscustomobject]@{ trace = $trace; fault_case = $Case }
    Add-Member -InputObject $inputStream -MemberType ScriptMethod -Name Close -Value {
        $this.trace.Add('stdin-close')
        if ($this.fault_case -eq 'close-error') { throw 'injected stdin close error' }
    }
    $process = [pscustomobject]@{
        StandardInput = $inputStream; trace = $trace; fault_case = $Case
        wait_calls = 0; ExitCode = 0; ExitTime = [DateTime]::UtcNow
    }
    Add-Member -InputObject $process -MemberType ScriptMethod -Name WaitForExit -Value {
        param([int]$Milliseconds)
        if ($Milliseconds -ne 1000) { throw 'Unexpected cleanup timeout' }
        $this.wait_calls = $this.wait_calls + 1
        $this.trace.Add('wait:' + $this.wait_calls)
        if ($this.wait_calls -eq 1) {
            if ($this.fault_case -eq 'initial-wait-error') { throw 'injected initial wait error' }
            return $false
        }
        return $this.fault_case -notin @('kill-error', 'final-wait-false')
    }
    Add-Member -InputObject $process -MemberType ScriptMethod -Name Kill -Value {
        $this.trace.Add('kill')
        if ($this.fault_case -eq 'kill-error') { throw 'injected kill error' }
    }
    Add-Member -InputObject $process -MemberType ScriptMethod -Name Dispose -Value {
        $this.trace.Add('dispose')
    }
    $probe = [pscustomobject]@{
        process = $process; started = $true; released = $false; process_id = 17
        exit_confirmed = $false; exit_code = $null; exit_filetime_utc = $null
        termination_requested = $false
    }
    $failures = [Collections.Generic.List[object]]::new()
    $primaryError = 'injected primary collection failure'
    Complete-MatrixProbe $probe $failures
    [ordered]@{
        diagnostic_only = $true; synthetic_cleanup_contract = $true
        native_gate_pass = $false; comparison_valid = $false; fault_case = $Case
        error = $primaryError; cleanup_errors = $failures.ToArray()
        trace = $trace.ToArray(); exit_confirmed = $probe.exit_confirmed
        termination_requested = $probe.termination_requested
        safe_for_follow_up = ($probe.exit_confirmed -and $failures.Count -eq 0)
    } | ConvertTo-Json -Depth 5 -Compress
}

if ($FunctionsOnly) { return }

if ($CleanupTestCase -ne 'none') {
    Invoke-MatrixCleanupFault $CleanupTestCase
    exit 0
}

function Receive-MatrixEvents {
    param($Context)
    foreach ($event in @(Get-Event | Where-Object {
        $_.SourceIdentifier -eq $Context.start_id -or $_.SourceIdentifier -eq $Context.stop_id
    })) {
        if ($Context.rows.Count -ge 4096) { throw 'Raw event limit exceeded' }
        $native = $event.SourceEventArgs.NewEvent
        $kind = if ($event.SourceIdentifier -eq $Context.start_id) { 'start' } else { 'stop' }
        $Context.rows.Add([pscustomobject]@{
            kind = $kind
            process_id = [int]$native.ProcessID
            parent_process_id = [int]$native.ParentProcessID
            name = [string]$native.ProcessName
            created_filetime_utc = [string]$native.TIME_CREATED
            received_ms = $Context.clock.ElapsedMilliseconds
        })
        Remove-Event -EventIdentifier $event.EventIdentifier -ErrorAction Stop
    }
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = ([Security.Principal.WindowsPrincipal]$identity).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
$identity.Dispose()
$prefix = 'cdr-creation-matrix-' + [Guid]::NewGuid().ToString('N')
$context = [pscustomobject]@{
    start_id = $prefix + '-start'
    stop_id = $prefix + '-stop'
    rows = [Collections.Generic.List[object]]::new()
    clock = [Diagnostics.Stopwatch]::StartNew()
}
$subscriptions = [Collections.Generic.List[string]]::new()
$probes = [Collections.Generic.List[object]]::new()
$cleanupErrors = [Collections.Generic.List[object]]::new()
$primaryError = $null
$windowCompleted = $false
$windowStarted = [DateTime]::UtcNow.ToFileTimeUtc().ToString()
try {
    foreach ($entry in @(
        @{ id = $context.start_id; class = 'Win32_ProcessStartTrace' },
        @{ id = $context.stop_id; class = 'Win32_ProcessStopTrace' }
    )) {
        $null = Register-WmiEvent -Class $entry.class -SourceIdentifier $entry.id -ErrorAction Stop
        $subscriptions.Add($entry.id)
    }
    foreach ($noWindow in @($true, $false)) {
        $process = [Diagnostics.Process]::new()
        $probe = [pscustomobject]@{
            process = $process; create_no_window = $noWindow; started = $false
            process_id = 0; creator_pid = $PID; created_filetime_utc = $null
            ready_task = $null; ready_checked = $false; ready = $false
            ready_ms = $null; released = $false; release_ms = $null
            exit_confirmed = $false; exit_code = $null; exit_filetime_utc = $null
            termination_requested = $false
        }
        $probes.Add($probe)
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = Join-Path $env:SystemRoot 'System32\cmd.exe'
        $info.Arguments = '/d /q /c "echo CDR_MATRIX_READY&set /p CDR_MATRIX_RELEASE=&exit /b 0"'
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $noWindow
        $info.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $info.RedirectStandardInput = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $process.StartInfo = $info
        if (-not $process.Start()) { throw 'Probe process did not start' }
        $probe.started = $true
        $probe.process_id = $process.Id
        $probe.created_filetime_utc = $process.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()
        $probe.ready_task = $process.StandardOutput.ReadLineAsync()
    }
    while ($context.clock.ElapsedMilliseconds -lt 5000) {
        Receive-MatrixEvents $context
        foreach ($probe in $probes) {
            if (-not $probe.ready_checked -and $probe.ready_task.IsCompleted) {
                $probe.ready_checked = $true
                $probe.ready = $probe.ready_task.GetAwaiter().GetResult() -ceq 'CDR_MATRIX_READY'
                $probe.ready_ms = $context.clock.ElapsedMilliseconds
            }
            if ($probe.ready -and -not $probe.released -and
                $context.clock.ElapsedMilliseconds -ge 2500 -and -not $probe.process.HasExited) {
                $probe.process.StandardInput.WriteLine('')
                $probe.process.StandardInput.Close()
                $probe.released = $true
                $probe.release_ms = $context.clock.ElapsedMilliseconds
            }
        }
        Start-Sleep -Milliseconds 20
    }
    Receive-MatrixEvents $context
    $windowCompleted = $true
} catch {
    $primaryError = $_.Exception.Message
} finally {
    $windowEnded = [DateTime]::UtcNow.ToFileTimeUtc().ToString()
    foreach ($probe in $probes) {
        Complete-MatrixProbe $probe $cleanupErrors
    }
    foreach ($sourceId in $subscriptions) {
        try { Unregister-Event -SourceIdentifier $sourceId -ErrorAction Stop } catch {
            $cleanupErrors.Add([pscustomobject]@{ stage = 'unsubscribe'; error = $_.Exception.Message })
        }
        try {
            foreach ($event in @(Get-Event | Where-Object { $_.SourceIdentifier -eq $sourceId })) {
                Remove-Event -EventIdentifier $event.EventIdentifier -ErrorAction Stop
            }
        } catch {
            $cleanupErrors.Add([pscustomobject]@{ stage = 'remove-event'; error = $_.Exception.Message })
        }
    }
}
$valid = $null -eq $primaryError -and $windowCompleted -and
    $cleanupErrors.Count -eq 0 -and $probes.Count -eq 2
$publicProbes = foreach ($probe in $probes) {
    if (-not ($probe.ready -and $probe.released -and $probe.exit_confirmed -and
        $probe.exit_code -eq 0 -and -not $probe.termination_requested)) { $valid = $false }
    [pscustomobject]@{
        create_no_window = $probe.create_no_window
        window_style = 'Hidden'
        process_id = $probe.process_id
        creator_pid = $probe.creator_pid
        created_filetime_utc = $probe.created_filetime_utc
        ready = $probe.ready
        ready_ms = $probe.ready_ms
        released = $probe.released
        release_ms = $probe.release_ms
        exit_confirmed = $probe.exit_confirmed
        exit_code = $probe.exit_code
        exit_filetime_utc = $probe.exit_filetime_utc
        termination_requested = $probe.termination_requested
        pid_start_count = @($context.rows | Where-Object {
            $_.kind -eq 'start' -and $_.process_id -eq $probe.process_id
        }).Count
        pid_stop_count = @($context.rows | Where-Object {
            $_.kind -eq 'stop' -and $_.process_id -eq $probe.process_id
        }).Count
    }
}
$result = [ordered]@{
    schema_version = 1
    diagnostic_only = $true
    native_gate_pass = $false
    comparison_valid = $valid
    admin = $admin
    host_pid = $PID
    parent_pid = $env:CDR_MATRIX_PARENT_PID
    acquisition_budget_ms = 5000
    release_after_ms = 2500
    window_started_filetime_utc = $windowStarted
    window_ended_filetime_utc = $windowEnded
    elapsed_with_cleanup_ms = $context.clock.ElapsedMilliseconds
    subscriptions_registered = $subscriptions.Count
    probes = @($publicProbes)
    raw_row_count = $context.rows.Count
    raw_rows = $context.rows.ToArray()
    error = $primaryError
    cleanup_errors = $cleanupErrors.ToArray()
}
$json = $result | ConvertTo-Json -Depth 8 -Compress
if ([Text.Encoding]::UTF8.GetByteCount($json) -gt 4194304) {
    $valid = $false
    $json = [ordered]@{
        diagnostic_only = $true; native_gate_pass = $false; comparison_valid = $false
        error = 'JSON output exceeds 4 MiB; no raw evidence can be claimed'
        original_error = $primaryError; raw_row_count = $context.rows.Count
        cleanup_errors = $cleanupErrors.ToArray()
    } | ConvertTo-Json -Depth 4 -Compress
}
[Console]::Out.WriteLine($json)
if ($valid) { exit 0 } else { exit 1 }
