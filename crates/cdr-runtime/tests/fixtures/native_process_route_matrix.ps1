# Diagnostic only: no product readiness override and no requested payload.
param([ValidateSet('capture', 'contracts', 'context')][string]$Mode = 'capture')
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'native_process_creation_matrix.ps1') -FunctionsOnly
Add-Type -Path (Join-Path $PSScriptRoot 'native_process_route_matrix.cs') -ReferencedAssemblies System.Management.dll, System.dll, System.Core.dll

function Test-RouteEvidence {
    param($Gate, $Probe, [string]$Begin, [string]$End, $Primary, [int]$CallbackErrors, [int]$CleanupErrors)
    return $Gate.Attempted -and $null -ne $Gate.Control -and $Probe.started -and $Probe.ready -and
        $Probe.released -and $Probe.exit_confirmed -and $Probe.exit_code -eq 0 -and
        -not $Probe.termination_requested -and $null -eq $Primary -and
        $CallbackErrors -eq 0 -and $CleanupErrors -eq 0 -and
        [CdrRouteRules]::Lifetime($Begin, $End, $Probe.created_filetime_utc, $Probe.exit_filetime_utc)
}

function New-ContractRow {
    param([uint32]$ProcessId = 20, [string]$Created = '110', [long]$Received = 500, [string]$Kind = 'start')
    $row = [CdrRouteRow]::new()
    $row.Kind = $Kind; $row.ProcessId = $ProcessId; $row.ParentProcessId = 2
    $row.Name = 'control.exe'; $row.CreatedTick = $Created
    $row.ReceivedUtcTick = '120'; $row.ReceivedMs = $Received
    return $row
}

function Invoke-RouteContracts {
    $results = [Collections.Generic.List[object]]::new()
    $row = New-ContractRow
    $gate = [CdrRouteGate]::new()
    $accepted = $gate.Observe(@($row), @($row), '100', 1, 500)
    $results.Add(@{name = 'matching_control_authorizes_once'; pass = ($accepted -and $gate.Attempted -and -not $gate.Observe(@($row), @($row), '100', 1, 501))})
    foreach ($case in @('absent', 'queue-only', 'mismatch', 'stale', 'late', 'future', 'wrong-kind')) {
        $queued = @($row); $direct = @($row)
        switch ($case) {
            'absent' { $queued = @(); $direct = @() }
            'queue-only' { $direct = @() }
            'mismatch' { $direct = @(New-ContractRow -ProcessId 21) }
            'stale' { $queued = @(New-ContractRow -Created '99'); $direct = $queued }
            'late' { $queued = @(New-ContractRow -Received 1000); $direct = $queued }
            'future' { $queued = @(New-ContractRow -Created '121'); $direct = $queued }
            'wrong-kind' { $queued = @(New-ContractRow -Kind 'stop'); $direct = $queued }
        }
        $rejected = [CdrRouteGate]::new()
        $early = $rejected.Observe($queued, $direct, '100', 1, 500)
        $expired = $rejected.Observe(@(), @(), '100', 1, 1000)
        $backfill = $rejected.Observe(@($row), @($row), '100', 1, 1100)
        $results.Add(@{name = $case; pass = (-not $early -and -not $expired -and -not $backfill -and -not $rejected.Attempted -and $rejected.Closed)})
    }
    $probe = [pscustomobject]@{started = $true; ready = $true; released = $true; exit_confirmed = $true
        exit_code = 0; termination_requested = $false; created_filetime_utc = '200'; exit_filetime_utc = '300'}
    $results.Add(@{name = 'valid_lifetime'; pass = (Test-RouteEvidence $gate $probe '100' '400' $null 0 0)})
    foreach ($exit in @('0', '199', '200', '401')) {
        $probe.exit_filetime_utc = $exit
        $results.Add(@{name = ('invalid_exit_' + $exit); pass = (-not (Test-RouteEvidence $gate $probe '100' '400' $null 0 0))})
    }
    $probe.exit_filetime_utc = '300'
    $results.Add(@{name = 'callback_error'; pass = (-not (Test-RouteEvidence $gate $probe '100' '400' $null 1 0))})
    $results.Add(@{name = 'cleanup_error'; pass = (-not (Test-RouteEvidence $gate $probe '100' '400' $null 0 1))})
    $results.Add(@{name = 'primary_error'; pass = (-not (Test-RouteEvidence $gate $probe '100' '400' 'fault' 0 0))})
    $probe.exit_confirmed = $false
    $results.Add(@{name = 'unconfirmed_exit'; pass = (-not (Test-RouteEvidence $gate $probe '100' '400' $null 0 0))})
    $probe.exit_confirmed = $true; $probe.termination_requested = $true
    $results.Add(@{name = 'forced_cleanup_is_not_observation'; pass = (-not (Test-RouteEvidence $gate $probe '100' '400' $null 0 0))})
    foreach ($case in @(
        @{name = 'token_boolean_false'; bytes = [byte[]]@(0); expected = $false},
        @{name = 'token_boolean_true'; bytes = [byte[]]@(1); expected = $true},
        @{name = 'token_dword_false'; bytes = [byte[]]@(0, 0, 0, 0); expected = $false},
        @{name = 'token_dword_true'; bytes = [byte[]]@(1, 0, 0, 0); expected = $true},
        @{name = 'token_dword_nonboolean'; bytes = [byte[]]@(2, 0, 0, 0); expected = $true},
        @{name = 'token_dword_high_bit'; bytes = [byte[]]@(0, 0, 0, 128); expected = $true}
    )) {
        $passed = $false
        try { $passed = [CdrRouteContext]::DecodeHasRestrictions($case.bytes) -eq $case.expected }
        catch { $passed = $false }
        $results.Add(@{name = $case.name; pass = $passed})
    }
    foreach ($case in @(
        @{name = 'token_null_rejected'; bytes = $null},
        @{name = 'token_empty_rejected'; bytes = [byte[]]@()},
        @{name = 'token_two_bytes_rejected'; bytes = [byte[]]@(0, 0)},
        @{name = 'token_three_bytes_rejected'; bytes = [byte[]]@(0, 0, 0)},
        @{name = 'token_five_bytes_rejected'; bytes = [byte[]]@(0, 0, 0, 0, 0)},
        @{name = 'token_boolean_two_rejected'; bytes = [byte[]]@(2)},
        @{name = 'token_boolean_max_rejected'; bytes = [byte[]]@(255)}
    )) {
        $passed = $false
        try { $null = [CdrRouteContext]::DecodeHasRestrictions($case.bytes) }
        catch { $passed = $_.Exception.InnerException -is [InvalidOperationException] }
        $results.Add(@{name = $case.name; pass = $passed})
    }
    return $results.ToArray()
}

if ($Mode -ne 'capture') {
    $contractRows = if ($Mode -eq 'contracts') { @(Invoke-RouteContracts) } else { @() }
    $result = [ordered]@{diagnostic_only = $true; native_gate_pass = $false; mode = $Mode
        contracts = $contractRows; observer_process = [CdrRouteContext]::Observer(); observer_thread = [CdrRouteContext]::Thread()}
    [Console]::Out.WriteLine(($result | ConvertTo-Json -Depth 10 -Compress))
    if (@($contractRows | Where-Object { -not $_.pass }).Count -gt 0) { exit 1 }
    exit 0
}

function Receive-RouteEvents {
    param($Context)
    foreach ($event in @(Get-Event | Where-Object { $_.SourceIdentifier -in @($Context.start_id, $Context.stop_id) })) {
        if ($Context.rows.Count -ge 4096) { throw 'queued raw row limit exceeded' }
        $value = $event.SourceEventArgs.NewEvent
        $row = [CdrRouteRow]::new()
        $row.Kind = if ($event.SourceIdentifier -eq $Context.start_id) { 'start' } else { 'stop' }
        $row.ProcessId = [uint32]$value.ProcessID; $row.ParentProcessId = [uint32]$value.ParentProcessID
        $row.Name = [string]$value.ProcessName; $row.CreatedTick = [string]$value.TIME_CREATED
        $row.ReceivedUtcTick = [DateTime]::UtcNow.ToFileTimeUtc().ToString()
        $row.ReceivedMs = $Context.clock.ElapsedMilliseconds
        $Context.rows.Add($row)
        Remove-Event -EventIdentifier $event.EventIdentifier -ErrorAction Stop
    }
}

$observerProcess = [CdrRouteContext]::Observer()
$observerThread = [CdrRouteContext]::Thread()
$prefix = 'cdr-route-' + [Guid]::NewGuid().ToString('N')
$clock = [Diagnostics.Stopwatch]::StartNew()
$begin = [DateTime]::UtcNow.ToFileTimeUtc().ToString()
$context = @{start_id = $prefix + '-start'; stop_id = $prefix + '-stop'
    rows = [Collections.Generic.List[CdrRouteRow]]::new(); clock = $clock}
$gate = [CdrRouteGate]::new()
$sources = [Collections.Generic.List[string]]::new()
$failures = [Collections.Generic.List[object]]::new()
$tap = [CdrRouteTap]::new($clock)
$primary = $null; $probeContext = $null; $registrationThread = $null
$probe = [pscustomobject]@{process = [Diagnostics.Process]::new(); started = $false; ready = $false
    released = $false; process_id = 0; created_filetime_utc = $null; ready_task = $null
    ready_ms = $null; release_ms = $null; exit_confirmed = $false; exit_code = $null
    exit_filetime_utc = $null; termination_requested = $false}
try {
    foreach ($entry in @(@{id = $context.start_id; class = 'Win32_ProcessStartTrace'},
                          @{id = $context.stop_id; class = 'Win32_ProcessStopTrace'})) {
        $null = Register-WmiEvent -Namespace root\cimv2 -Class $entry.class -SourceIdentifier $entry.id -ErrorAction Stop
        $sources.Add($entry.id)
    }
    $tap.Start()
    $registrationThread = [CdrRouteContext]::Thread()
    while ($clock.ElapsedMilliseconds -lt 5000) {
        Receive-RouteEvents $context
        if ($gate.Observe($context.rows.ToArray(), $tap.Rows.ToArray(), $begin, $PID, $clock.ElapsedMilliseconds)) {
            $info = [Diagnostics.ProcessStartInfo]::new()
            $info.FileName = Join-Path $env:SystemRoot 'System32\cmd.exe'
            $info.Arguments = '/d /q /c "echo CDR_MATRIX_READY&set /p CDR_MATRIX_RELEASE=&exit /b 0"'
            $info.UseShellExecute = $false; $info.CreateNoWindow = $true
            $info.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
            $info.RedirectStandardInput = $true; $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
            $probe.process.StartInfo = $info
            if (-not $probe.process.Start()) { throw 'route probe did not start' }
            $probe.started = $true; $probe.process_id = $probe.process.Id
            $probe.created_filetime_utc = $probe.process.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()
            $probe.ready_task = $probe.process.StandardOutput.ReadLineAsync()
            $probeContext = [CdrRouteContext]::Process($probe.process.Handle)
        }
        if ($probe.started -and -not $probe.ready -and $probe.ready_task.IsCompleted) {
            if ($probe.ready_task.GetAwaiter().GetResult() -cne 'CDR_MATRIX_READY') { throw 'invalid route probe ready signal' }
            $probe.ready = $true; $probe.ready_ms = $clock.ElapsedMilliseconds
        }
        if ($probe.ready -and -not $probe.released -and $clock.ElapsedMilliseconds -ge 2500) {
            $probe.process.StandardInput.WriteLine('')
            $probe.process.StandardInput.Close()
            $probe.released = $true; $probe.release_ms = $clock.ElapsedMilliseconds
        }
        Start-Sleep -Milliseconds 10
    }
    Receive-RouteEvents $context
} catch { $primary = $_.Exception.Message }
finally {
    $end = [DateTime]::UtcNow.ToFileTimeUtc().ToString()
    $acquisitionMs = $clock.ElapsedMilliseconds
    Complete-MatrixProbe $probe $failures
    $tap.Dispose()
    foreach ($message in $tap.CleanupFaults.ToArray()) { Add-MatrixCleanupFailure $failures 'direct-watcher' 0 $message }
    foreach ($source in $sources) {
        try { Unregister-Event -SourceIdentifier $source -ErrorAction Stop }
        catch { Add-MatrixCleanupFailure $failures 'unregister' 0 $_.Exception.Message }
        try { Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue | Remove-Event -ErrorAction Stop }
        catch { Add-MatrixCleanupFailure $failures 'remove-events' 0 $_.Exception.Message }
    }
}
$queueRows = $context.rows.ToArray(); $directRows = $tap.Rows.ToArray()
$callbacks = $tap.Faults.ToArray()
$valid = Test-RouteEvidence $gate $probe $begin $end $primary $callbacks.Count $failures.Count
$counts = foreach ($route in @('queue', 'direct')) {
    $rows = if ($route -eq 'queue') { $queueRows } else { $directRows }
    [pscustomobject]@{route = $route
        start = @($rows | Where-Object { $_.ProcessId -eq $probe.process_id -and $_.Kind -eq 'start' -and $_.ReceivedMs -lt 5000 }).Count
        stop = @($rows | Where-Object { $_.ProcessId -eq $probe.process_id -and $_.Kind -eq 'stop' -and $_.ReceivedMs -lt 5000 }).Count}
}
$status = if ($null -ne $primary -or $callbacks.Count -gt 0 -or $failures.Count -gt 0) { 'ERROR' } elseif ($valid) { 'OBSERVED' } else { 'INCONCLUSIVE' }
$result = [ordered]@{schema_version = 1; diagnostic_only = $true; native_gate_pass = $false
    status = $status; comparison_valid = $valid; host_pid = $PID; parent_pid = $env:CDR_ROUTE_PARENT_PID
    acquisition_budget_ms = 5000; positive_control_deadline_ms = 1000; release_after_ms = 2500
    acquisition_elapsed_ms = $acquisitionMs; window_started_filetime_utc = $begin; window_ended_filetime_utc = $end
    positive_control = $gate.Control; start_attempted = $gate.Attempted
    stop_positive_control = [CdrRouteRules]::Common($queueRows, $directRows, $begin, $PID, 'stop', 5000)
    observer_process = $observerProcess; observer_thread = $observerThread; registration_thread = $registrationThread
    probe_context = $probeContext
    probe = [ordered]@{process_id = $probe.process_id; creator_pid = $PID; started = $probe.started; ready = $probe.ready
        created_filetime_utc = $probe.created_filetime_utc; ready_ms = $probe.ready_ms; released = $probe.released
        release_ms = $probe.release_ms; exit_confirmed = $probe.exit_confirmed; exit_code = $probe.exit_code
        exit_filetime_utc = $probe.exit_filetime_utc; termination_requested = $probe.termination_requested}
    counts = @($counts); queue_rows = $queueRows; direct_rows = $directRows; callback_errors = $callbacks
    cleanup_errors = $failures.ToArray(); error = $primary
    correlation = 'PID diagnostic counts only, not an ownership or independent-lineage attestation'
    safe_for_follow_up = ($null -eq $primary -and $callbacks.Count -eq 0 -and $failures.Count -eq 0 -and (-not $probe.started -or $probe.exit_confirmed))}
$json = $result | ConvertTo-Json -Depth 12 -Compress
if ([Text.Encoding]::UTF8.GetByteCount($json) -gt 4194304) { throw 'route JSON exceeds 4 MiB; no passing record was produced' }
[Console]::Out.WriteLine($json)
if ($status -eq 'ERROR') { exit 1 } else { exit 0 }
