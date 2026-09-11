# Rust-only cutover support; loaded by codex-discord-runtime-cutover.ps1.
function Get-ProcessIdentityById {
    param([int]$ProcessId)
    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) { return '' }
    try {
        $ticks = $process.StartTime.ToUniversalTime().Ticks
        $ticks -= ($ticks % 10)
        return "$ProcessId|$ticks"
    } catch {
        return ''
    }
}

function Test-ProcessIdentityAlive {
    param([string]$Identity)
    if ($Identity -notmatch '^(\d+)\|(\d+)$') { return $false }
    return (Get-ProcessIdentityById -ProcessId ([int]$Matches[1])) -eq $Identity
}

function Get-VerifiedRustProcess {
    if (-not (Test-Path -LiteralPath $RustLock -PathType Leaf)) { return $null }
    $text = Get-Content -LiteralPath $RustLock -Raw -ErrorAction SilentlyContinue
    if ($text -notmatch '(?m)^pid=(\d+)$') { return $null }
    $process = Get-Process -Id ([int]$Matches[1]) -ErrorAction SilentlyContinue
    if ($null -eq $process) { return $null }
    try {
        $actualPath = [string]$process.Path
        if ([string]::IsNullOrWhiteSpace($actualPath)) { return $null }
        if (-not [IO.Path]::GetFullPath($actualPath).Equals(
            $BinaryPath,
            [StringComparison]::OrdinalIgnoreCase
        )) { return $null }
        return $process
    } catch {
        return $null
    }
}

function Get-RustIdentity {
    $process = Get-VerifiedRustProcess
    if ($null -eq $process) { return '' }
    $ticks = $process.StartTime.ToUniversalTime().Ticks
    $ticks -= ($ticks % 10)
    return "$([int]$process.Id)|$ticks"
}

function Get-VerifiedRustHeartbeatState {
    param($Process)
    if (-not (Test-Path -LiteralPath $RustHeartbeat -PathType Leaf)) {
        return 'heartbeat_missing'
    }
    $text = Get-Content -LiteralPath $RustHeartbeat -Raw -ErrorAction SilentlyContinue
    if ($text -notmatch '(?m)^pid=(\d+)$') { return 'heartbeat_pid_invalid' }
    if ([int]$Matches[1] -ne [int]$Process.Id) { return 'heartbeat_pid_mismatch' }
    if ($text -notmatch '(?m)^updated_at=(\d+)$') { return 'heartbeat_timestamp_invalid' }
    try {
        $updated = [DateTimeOffset]::FromUnixTimeSeconds([long]$Matches[1])
        $age = ([DateTimeOffset]::UtcNow - $updated).TotalSeconds
        if ($age -lt 0) { return 'heartbeat_future' }
        if ($updated.ToUnixTimeSeconds() -lt ([DateTimeOffset]$Process.StartTime.ToUniversalTime()).ToUnixTimeSeconds()) {
            return 'heartbeat_prestart'
        }
        if ($age -gt $HeartbeatMaxAgeSeconds) { return 'heartbeat_stale' }
    } catch {
        return 'heartbeat_timestamp_invalid'
    }
    return 'healthy'
}

function Wait-RustHealthy {
    $identity = Get-RustIdentity
    if ([string]::IsNullOrWhiteSpace($identity)) {
        throw 'Rust runtime lock was not owned by a verified process.'
    }
    $process = Get-VerifiedRustProcess
    $bootstrapDeadline = $process.StartTime.AddSeconds($HeartbeatBootstrapGraceSeconds)
    $healthySince = $null
    while ($true) {
        if ((Get-RustIdentity) -ne $identity) {
            throw 'Rust runtime exited or was replaced during cutover observation.'
        }
        $process = Get-VerifiedRustProcess
        $state = Get-VerifiedRustHeartbeatState -Process $process
        $now = Get-Date
        if ($state -eq 'healthy') {
            if ($null -eq $healthySince) { $healthySince = $now }
            if (($now - $healthySince).TotalSeconds -ge $ObserveSeconds) { return }
        } elseif ($null -ne $healthySince) {
            throw "Rust heartbeat failed during cutover observation: state=$state"
        } elseif ($now -gt $bootstrapDeadline) {
            throw (
                "Rust heartbeat did not become ready within bootstrap grace: " +
                "state=$state grace_seconds=$HeartbeatBootstrapGraceSeconds"
            )
        }
        Start-Sleep -Milliseconds 250
    }
}

function Assert-RustCutoverPreflight {
    foreach ($required in @($BinaryPath, $EnvPath, $RustWatchdog)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw "Rust cutover prerequisite was not found: $required"
        }
    }
    & $BinaryPath --check-config --env $EnvPath
    if ($LASTEXITCODE -ne 0) {
        throw "Rust configuration preflight failed with exit code $LASTEXITCODE"
    }
}

function Publish-Mode {
    param([ValidateSet('rust')][string]$Value)
    Write-AtomicUtf8 -Path $ModePath -Content "$Value`n"
}

function Backup-Store {
    & $BinaryPath --backup-store --env $EnvPath
    if ($LASTEXITCODE -ne 0) {
        throw "Rust online store backup failed with exit code $LASTEXITCODE"
    }
}

function Stop-Rust {
    $control = Enter-CdrControl -Root $RepoRoot
    try {
        Assert-CdrNoPendingRestart -Root $RepoRoot
        if ($null -eq (Get-VerifiedRustProcess)) { return }
        $script:CutoverStopText = "cutover_stop_owner=$CutoverIdentity`n"
        Write-NewCdrMarker -Path $RustStop -Text $script:CutoverStopText
    } finally { $control.Dispose() }
    & $RustWatchdog -RepoRoot $RepoRoot -BinaryPath $BinaryPath -EnvPath $EnvPath
    if ($LASTEXITCODE -ne 0 -or $null -ne (Get-VerifiedRustProcess)) {
        throw 'Verified Rust runtime did not stop cleanly.'
    }
    $control = Enter-CdrControl -Root $RepoRoot
    try {
        if ($null -ne (Get-VerifiedRustProcess)) { throw 'Rust instance appeared during cutover stop cleanup' }
        Assert-CdrMarkerOwner -Path $RustStop -Text $script:CutoverStopText
        if ([IO.File]::Exists($RustStop)) { [IO.File]::Delete($RustStop) }
    } finally { $control.Dispose() }
}

function Start-Rust {
    $control = Enter-CdrControl -Root $RepoRoot
    try {
        if (Test-Path -LiteralPath $RustRestart) {
            throw 'Existing restart fence preserved; cutover startup refused'
        }
        Assert-CdrNoPendingRestart -Root $RepoRoot
        if (Test-Path -LiteralPath $RustStop) {
            if ([string]::IsNullOrWhiteSpace($script:CutoverStopText)) {
                throw 'Unowned stop marker preserved; cutover startup refused'
            }
            Assert-CdrMarkerOwner -Path $RustStop -Text $script:CutoverStopText
            [IO.File]::Delete($RustStop)
        }
    } finally { $control.Dispose() }
    & $RustWatchdog -RepoRoot $RepoRoot -BinaryPath $BinaryPath -EnvPath $EnvPath
    if ($LASTEXITCODE -ne 0 -or $null -eq (Get-VerifiedRustProcess)) {
        throw 'Rust watchdog did not start a verified runtime.'
    }
}

function Get-CurrentRuntime {
    if (Test-Path -LiteralPath $ModePath -PathType Leaf) {
        $mode = (Get-Content -LiteralPath $ModePath -Raw).Trim().ToLowerInvariant()
        if ($mode -eq 'rust') { return $mode }
        throw "Unsupported runtime mode '$mode'; runtime selection was not changed."
    }
    if ($null -ne (Get-VerifiedRustProcess)) { return 'rust' }
    throw 'Current runtime could not be determined safely.'
}

function Test-TargetHealthy {
    param([ValidateSet('rust')][string]$Target)
    $process = Get-VerifiedRustProcess
    if ($null -eq $process) { return $false }
    return (Get-VerifiedRustHeartbeatState -Process $process) -eq 'healthy'
}
