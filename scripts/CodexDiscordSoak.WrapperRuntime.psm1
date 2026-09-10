Set-StrictMode -Version Latest

function Resolve-CodexSoakRootedPath {
    param([string]$Base, [string]$Path)
    if ([IO.Path]::IsPathRooted($Path)) { return [IO.Path]::GetFullPath($Path) }
    [IO.Path]::GetFullPath((Join-Path $Base $Path))
}

function Get-CodexSoakFileSha256 {
    param([string]$Path)
    $stream = [IO.File]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '') }
    finally { $sha.Dispose(); $stream.Dispose() }
}

function Get-CodexSoakDisabledMarker {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Offline soak requires the existing operator disabled marker: $Path"
    }
    $item = Get-Item -LiteralPath $Path
    [pscustomobject]@{ Hash = Get-CodexSoakFileSha256 $Path; Length = [long]$item.Length }
}

function Assert-CodexSoakDisabledMarker {
    param([string]$Path, [object]$Before)
    $after = Get-CodexSoakDisabledMarker $Path
    if ($after.Hash -ne $Before.Hash -or $after.Length -ne $Before.Length) {
        throw "Operator disabled marker changed during offline soak: $Path"
    }
    $after
}

function Assert-CodexSoakRuntimeStopped {
    param([string]$Root)
    $lock = Join-Path $Root '.codex_discord_rust.runtime.lock'
    if (Test-Path -LiteralPath $lock -PathType Leaf) {
        $text = [IO.File]::ReadAllText($lock)
        if ($text -match '(?m)^pid=(\d+)$' -and
            (Get-Process -Id ([int]$Matches[1]) -ErrorAction SilentlyContinue)) {
            throw "Offline soak requires the Rust runtime stopped; live lock PID $($Matches[1])"
        }
    }
    $expected = [IO.Path]::GetFullPath((Join-Path $Root 'target\release\cdr-runtime.exe'))
    foreach ($process in @(Get-Process -Name cdr-runtime -ErrorAction SilentlyContinue)) {
        try { $actual = [IO.Path]::GetFullPath([string]$process.Path) }
        catch { throw "Cannot verify cdr-runtime PID $($process.Id) is stopped" }
        if ($actual.Equals($expected, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Offline soak requires cdr-runtime stopped: PID $($process.Id)"
        }
    }
}

function Test-CodexSoakWithinExitDeadline {
    [OutputType([bool])]
    param(
        [Parameter(Mandatory = $true)][TimeSpan]$Elapsed,
        [Parameter(Mandatory = $true)][long]$DurationSeconds,
        [Parameter(Mandatory = $true)][long]$GraceSeconds
    )
    if ($DurationSeconds -lt 0) {
        throw [ArgumentOutOfRangeException]::new(
            'DurationSeconds', $DurationSeconds,
            'Soak exit deadline duration must be non-negative'
        )
    }
    if ($GraceSeconds -lt 0) {
        throw [ArgumentOutOfRangeException]::new(
            'GraceSeconds', $GraceSeconds,
            'Soak exit deadline grace must be non-negative'
        )
    }
    if ($Elapsed.Ticks -lt 0) {
        throw [ArgumentOutOfRangeException]::new(
            'Elapsed', $Elapsed,
            'Soak exit deadline elapsed time must be non-negative'
        )
    }
    [decimal]$deadlineTicks = (
        [decimal]$DurationSeconds + [decimal]$GraceSeconds
    ) * [decimal][TimeSpan]::TicksPerSecond
    if ($deadlineTicks -gt [decimal][long]::MaxValue) {
        throw [OverflowException]::new(
            'Soak exit deadline cannot be represented as TimeSpan ticks'
        )
    }
    $Elapsed.Ticks -le [long]$deadlineTicks
}

function ConvertTo-CodexSoakNativeArgument {
    param([string]$Value)
    $escaped = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    '"' + $escaped + '"'
}

function Add-CodexSoakMemorySample {
    param(
        [Diagnostics.Process]$Process, [int]$ExpectedPid, [long]$ExpectedStartTicks,
        [datetime]$StartedAt, [double]$ElapsedSeconds, [string]$EvidenceId,
        [Collections.Generic.List[object]]$Samples, [IO.StreamWriter]$Writer
    )
    $Process.Refresh()
    if ($Process.HasExited) { return }
    $actualStart = $Process.StartTime.ToUniversalTime()
    if ($Process.Id -ne $ExpectedPid -or $actualStart.Ticks -ne $ExpectedStartTicks) {
        throw "Owned soak child identity changed while sampling PID $ExpectedPid"
    }
    $sample = [pscustomobject]@{
        ElapsedSeconds = $ElapsedSeconds
        WorkingSetBytes = [long]$Process.WorkingSet64
    }
    $null = $Samples.Add($sample)
    $record = [ordered]@{
        schema = 'cdr.windows-soak.memory-sample.v1'; evidence_id = $EvidenceId
        harness_pid = $ExpectedPid; harness_started_at_utc = $StartedAt.ToString('o')
        sampled_at_utc = [datetime]::UtcNow.ToString('o'); elapsed_seconds = $ElapsedSeconds
        working_set_bytes = $sample.WorkingSetBytes
    }
    $Writer.WriteLine(($record | ConvertTo-Json -Compress))
    $Writer.Flush()
}

function Get-CodexSoakMemoryRegression {
    param([object[]]$Samples, [double]$Warmup)
    $selected = @($Samples | Where-Object { $_.ElapsedSeconds -ge $Warmup })
    if ($selected.Count -lt 2) {
        throw "Need at least two post-warmup memory samples; found $($selected.Count)"
    }
    [double]$sumX = 0; [double]$sumY = 0; [double]$sumXX = 0; [double]$sumXY = 0
    foreach ($sample in $selected) {
        $x = [double]$sample.ElapsedSeconds / 3600.0
        $y = [double]$sample.WorkingSetBytes
        $sumX += $x; $sumY += $y; $sumXX += $x * $x; $sumXY += $x * $y
    }
    $n = [double]$selected.Count
    $denominator = $n * $sumXX - $sumX * $sumX
    if ([math]::Abs($denominator) -lt 1e-18) { throw 'Memory sample times have no span' }
    [pscustomobject]@{
        Count = $selected.Count
        SlopeBytesPerHour = ($n * $sumXY - $sumX * $sumY) / $denominator
    }
}

function Get-CodexSoakJsonLineCount {
    param([string]$Path, [long]$ExpectedSeed)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Harness events JSONL is missing: $Path"
    }
    $reader = [IO.StreamReader]::new($Path, [Text.UTF8Encoding]::new($false, $true), $false)
    $count = 0; $lineNumber = 0
    try {
        while ($null -ne ($line = $reader.ReadLine())) {
            $lineNumber++
            if ([string]::IsNullOrWhiteSpace($line)) { continue }
            try { $record = $line | ConvertFrom-Json }
            catch { throw "Invalid harness JSONL at line $lineNumber`: $($_.Exception.Message)" }
            if ($record.schema -ne 'cdr.offline-soak.progress.v1' -or
                $record.mode -ne 'offline_fake_replay' -or $record.seed -ne $ExpectedSeed) {
                throw "Harness JSONL identity mismatch at line $lineNumber"
            }
            $count++
        }
    } finally { $reader.Dispose() }
    if ($count -eq 0) { throw 'Harness events JSONL contains no records' }
    $count
}

function Write-CodexSoakCapturedOutput {
    param([object]$Task, [string]$Path, [Text.Encoding]$Encoding)
    if ($null -eq $Task) { return }
    if (-not $Task.Wait(1000)) { throw "Harness output did not drain: $Path" }
    [IO.File]::WriteAllText($Path, $Task.Result, $Encoding)
}

Export-ModuleMember -Function @(
    'Resolve-CodexSoakRootedPath', 'Get-CodexSoakDisabledMarker',
    'Assert-CodexSoakDisabledMarker', 'Assert-CodexSoakRuntimeStopped',
    'Test-CodexSoakWithinExitDeadline',
    'ConvertTo-CodexSoakNativeArgument', 'Add-CodexSoakMemorySample',
    'Get-CodexSoakMemoryRegression', 'Get-CodexSoakJsonLineCount',
    'Write-CodexSoakCapturedOutput'
)
