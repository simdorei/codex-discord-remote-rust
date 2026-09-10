[CmdletBinding()]
param(
    [ValidateSet('rust', 'python')]
    [string]$Runtime = 'rust',
    [string]$RepoRoot,
    [string]$BinaryPath,
    [string]$EnvPath,
    [int]$ObserveSeconds = 30,
    [int]$HeartbeatMaxAgeSeconds = 45,
    [int]$HeartbeatBootstrapGraceSeconds = 120,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) { $RepoRoot = $PSScriptRoot }
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
if ([string]::IsNullOrWhiteSpace($BinaryPath)) {
    $BinaryPath = Join-Path $RepoRoot 'target\release\cdr-runtime.exe'
}
if ([string]::IsNullOrWhiteSpace($EnvPath)) { $EnvPath = Join-Path $RepoRoot '.env' }
$BinaryPath = [IO.Path]::GetFullPath($BinaryPath)
$EnvPath = [IO.Path]::GetFullPath($EnvPath)
if ($ObserveSeconds -lt 0) { throw 'ObserveSeconds cannot be negative.' }
if ($HeartbeatMaxAgeSeconds -lt 1) { throw 'HeartbeatMaxAgeSeconds must be at least 1.' }
if ($HeartbeatBootstrapGraceSeconds -lt 0) {
    throw 'HeartbeatBootstrapGraceSeconds cannot be negative.'
}

$ModePath = Join-Path $RepoRoot '.codex_discord_runtime'
$CutoverStatePath = Join-Path $RepoRoot '.codex_discord_runtime.cutover'
$PythonScript = Join-Path $RepoRoot 'codex_discord_bot.py'
$PythonLock = Join-Path $RepoRoot '.codex_discord_bot.runtime.lock'
$PythonStop = Join-Path $RepoRoot '.codex_discord_bot.stop'
$PythonWatchdog = Join-Path $RepoRoot 'codex-discord-watchdog.ps1'
$PythonRuntime = Join-Path $PSScriptRoot 'codex-discord-python-runtime.ps1'
$PythonBackupScript = Join-Path $PSScriptRoot 'scripts\backup_sqlite_store.py'
$IdentityRuntime = Join-Path $RepoRoot 'codex-discord-watchdog-identity-runtime.ps1'
$RustLock = Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'
$RustStop = Join-Path $RepoRoot '.codex_discord_rust.stop'
$RustRestart = Join-Path $RepoRoot '.codex_discord_rust.restart'
$RustHeartbeat = Join-Path $RepoRoot '.codex_discord_rust.heartbeat'
$RustWatchdog = Join-Path $RepoRoot 'codex-discord-rust-watchdog.ps1'
$DisablePath = Join-Path $RepoRoot '.codex_discord_bot.disabled'
. (Join-Path $RepoRoot 'codex-discord-rust-control.ps1')
. (Join-Path $PSScriptRoot 'scripts/CdrCutoverCompletion.ps1')
$CutoverRecoveryHandled = $false
$CutoverRecoveryPreviewed = $false
$PythonBackupExecutable = ''
$PythonBackupDatabase = ''

if (-not (Test-Path -LiteralPath $IdentityRuntime -PathType Leaf)) {
    throw "Cutover prerequisite was not found: $IdentityRuntime"
}
. $IdentityRuntime

$self = [Diagnostics.Process]::GetCurrentProcess()
try {
    $selfTicks = $self.StartTime.ToUniversalTime().Ticks
    $selfTicks -= ($selfTicks % 10)
    $CutoverIdentity = "$PID|$selfTicks"
} finally {
    $self.Dispose()
}

function Write-AtomicUtf8 {
    param([string]$Path, [string]$Content)
    $temporary = "$Path.tmp.$PID"
    $utf8 = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText($temporary, $Content, $utf8)
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Read-KeyValueFile {
    param([string]$Path)
    $values = @{}
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $values }
    foreach ($line in (Get-Content -LiteralPath $Path -ErrorAction Stop)) {
        if ([string]$line -match '^([^=]+)=(.*)$') {
            $values[[string]$Matches[1]] = [string]$Matches[2]
        }
    }
    return $values
}

function Get-CutoverEnvValue {
    param([string]$Name)
    $processValue = [Environment]::GetEnvironmentVariable($Name, 'Process')
    if (-not [string]::IsNullOrWhiteSpace($processValue)) {
        return $processValue.Trim().Trim('"').Trim("'")
    }
    if (-not (Test-Path -LiteralPath $EnvPath -PathType Leaf)) { return '' }
    foreach ($line in (Get-Content -LiteralPath $EnvPath -Encoding UTF8 -ErrorAction Stop)) {
        $trimmed = ([string]$line).Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed) -or $trimmed.StartsWith('#')) {
            continue
        }
        $separator = $trimmed.IndexOf('=')
        if ($separator -lt 1) { continue }
        $key = $trimmed.Substring(0, $separator).Trim()
        if (-not $key.Equals($Name, [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        return $trimmed.Substring($separator + 1).Trim().Trim('"').Trim("'")
    }
    return ''
}

function Resolve-CutoverPath {
    param([string]$Value)
    $expanded = [Environment]::ExpandEnvironmentVariables($Value)
    if ([IO.Path]::IsPathRooted($expanded)) {
        return [IO.Path]::GetFullPath($expanded)
    }
    return [IO.Path]::GetFullPath((Join-Path $RepoRoot $expanded))
}

function Get-ConfiguredStorePath {
    $configured = Get-CutoverEnvValue -Name 'CODEX_DISCORD_MIRROR_DB'
    if (-not [string]::IsNullOrWhiteSpace($configured)) {
        return Resolve-CutoverPath -Value $configured
    }
    $configuredRoot = Get-CutoverEnvValue -Name 'CODEX_DISCORD_ROOT'
    $storeRoot = if ([string]::IsNullOrWhiteSpace($configuredRoot)) {
        $RepoRoot
    } else {
        Resolve-CutoverPath -Value $configuredRoot
    }
    return Join-Path $storeRoot 'discord_mirror.sqlite'
}

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

function Get-PythonIdentity {
    return Get-CodexBotProcessIdentity -BotScript $PythonScript -RuntimeLockPath $PythonLock
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

function Invoke-PythonWatchdog {
    if (-not (Test-Path -LiteralPath $PythonWatchdog -PathType Leaf)) {
        throw "Python recovery watchdog was not found: $PythonWatchdog"
    }
    $previous = [Environment]::GetEnvironmentVariable('CODEX_DISCORD_RUNTIME', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('CODEX_DISCORD_RUNTIME', 'python', 'Process')
        & $PythonWatchdog
        if ($LASTEXITCODE -ne 0) {
            throw "Python watchdog failed with exit code $LASTEXITCODE"
        }
    } finally {
        [Environment]::SetEnvironmentVariable('CODEX_DISCORD_RUNTIME', $previous, 'Process')
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

function Assert-PythonRollbackPreflight {
    foreach ($required in @(
        $PythonScript,
        $PythonWatchdog,
        $PythonRuntime,
        $PythonBackupScript
    )) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw "Python rollback prerequisite was not found: $required"
        }
    }
    . $PythonRuntime
    $script:PythonBackupExecutable = Resolve-CodexRuntimePythonExecutable -RepoRoot $RepoRoot
    $script:PythonBackupDatabase = Get-ConfiguredStorePath
    if (-not (Test-Path -LiteralPath $PythonBackupDatabase -PathType Leaf)) {
        throw "Python rollback store database was not found: $PythonBackupDatabase"
    }
}

function Backup-StoreWithPython {
    & $PythonBackupExecutable $PythonBackupScript --source $PythonBackupDatabase
    if ($LASTEXITCODE -ne 0) {
        throw "Python online store backup failed with exit code $LASTEXITCODE"
    }
}

function Publish-Mode {
    param([ValidateSet('rust', 'python')][string]$Value)
    Write-AtomicUtf8 -Path $ModePath -Content "$Value`n"
}

function Backup-Store {
    & $BinaryPath --backup-store --env $EnvPath
    if ($LASTEXITCODE -ne 0) {
        throw "Rust online store backup failed with exit code $LASTEXITCODE"
    }
}

function Stop-Python {
    $identity = Get-PythonIdentity
    if ([string]::IsNullOrWhiteSpace($identity)) { return }
    Write-AtomicUtf8 -Path $PythonStop -Content "identity=$identity`n"
    Invoke-PythonWatchdog
    if (-not [string]::IsNullOrWhiteSpace((Get-PythonIdentity))) {
        throw 'Verified Python runtime remained alive after stop.'
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

function Start-Python {
    $control = Enter-CdrControl -Root $RepoRoot
    try { Assert-CdrNoPendingRestart -Root $RepoRoot }
    finally { $control.Dispose() }
    Invoke-PythonWatchdog
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $deadline) {
        if (-not [string]::IsNullOrWhiteSpace((Get-PythonIdentity))) { return }
        Start-Sleep -Milliseconds 500
    }
    throw 'Python watchdog did not start a verified rollback runtime.'
}

function Get-CurrentRuntime {
    if (Test-Path -LiteralPath $ModePath -PathType Leaf) {
        $mode = (Get-Content -LiteralPath $ModePath -Raw).Trim().ToLowerInvariant()
        if ($mode -in @('python', 'rust')) { return $mode }
    }
    $pythonRunning = -not [string]::IsNullOrWhiteSpace((Get-PythonIdentity))
    $rustRunning = $null -ne (Get-VerifiedRustProcess)
    if ($pythonRunning -and -not $rustRunning) { return 'python' }
    if ($rustRunning -and -not $pythonRunning) { return 'rust' }
    throw 'Current runtime could not be determined safely.'
}

function Get-CutoverState {
    if (-not (Test-Path -LiteralPath $CutoverStatePath -PathType Leaf)) { return $null }
    $values = Read-KeyValueFile -Path $CutoverStatePath
    foreach ($key in @('transaction_id', 'owner_identity', 'source_runtime', 'target_runtime', 'phase')) {
        if ([string]::IsNullOrWhiteSpace([string]$values[$key])) {
            throw "Cutover recovery state is invalid: missing $key"
        }
    }
    if ($values.source_runtime -notin @('python', 'rust')) {
        throw 'Cutover recovery state has an invalid source runtime.'
    }
    if ($values.target_runtime -notin @('python', 'rust')) {
        throw 'Cutover recovery state has an invalid target runtime.'
    }
    return [pscustomobject]@{
        TransactionId = [string]$values.transaction_id
        OwnerIdentity = [string]$values.owner_identity
        SourceRuntime = [string]$values.source_runtime
        TargetRuntime = [string]$values.target_runtime
        Phase = [string]$values.phase
        TargetIdentity = [string]$values.target_identity
        CompletionRuntime = [string]$values.completion_runtime
    }
}

function Set-CutoverPhase {
    param($Transaction, [string]$Phase)
    $Transaction.OwnerIdentity = $CutoverIdentity
    $Transaction.Phase = $Phase
    $content = (
        "transaction_id=$($Transaction.TransactionId)`n" +
        "owner_identity=$($Transaction.OwnerIdentity)`n" +
        "source_runtime=$($Transaction.SourceRuntime)`n" +
        "target_runtime=$($Transaction.TargetRuntime)`n" +
        "phase=$Phase`n" +
        "target_identity=$($Transaction.TargetIdentity)`n" +
        "completion_runtime=$($Transaction.CompletionRuntime)`n" +
        "updated_at=$((Get-Date).ToUniversalTime().ToString('o'))`n"
    )
    Write-AtomicUtf8 -Path $CutoverStatePath -Content $content
}

function Get-DisableOwner {
    if (-not (Test-Path -LiteralPath $DisablePath -PathType Leaf)) { return $null }
    return Read-KeyValueFile -Path $DisablePath
}

function Enter-CutoverMaintenance {
    param($Transaction)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
    Assert-CdrNoPendingRestart -Root $RepoRoot
    $owner = Get-DisableOwner
    if ($null -ne $owner) {
        if (
            $owner.kind -notin @('cutover', 'cutover_recovery') -or
            $owner.transaction_id -ne $Transaction.TransactionId
        ) {
            throw "Runtime is disabled by an operator-owned marker: $DisablePath"
        }
        return
    }
    Write-NewCdrMarker -Path $DisablePath -Text (
        "kind=cutover`n" +
        "transaction_id=$($Transaction.TransactionId)`n" +
        "cutover_identity=$CutoverIdentity`n"
    )
    } finally { $control.Dispose() }
}

function Exit-CutoverMaintenance {
    param($Transaction)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
    $owner = Get-DisableOwner
    if ($null -eq $owner) { return }
    if (
        $owner.kind -notin @('cutover', 'cutover_recovery') -or
        $owner.transaction_id -ne $Transaction.TransactionId
    ) {
        throw 'Cutover maintenance marker ownership changed.'
    }
    Remove-Item -LiteralPath $DisablePath -Force
    } finally { $control.Dispose() }
}

function Ensure-CutoverRecoveryDisabled {
    param($Transaction)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
    Assert-CdrNoPendingRestart -Root $RepoRoot
    if (Test-Path -LiteralPath $DisablePath -PathType Leaf) { return }
    Write-NewCdrMarker -Path $DisablePath -Text (
        "kind=cutover_recovery`n" +
        "transaction_id=$($Transaction.TransactionId)`n" +
        "recovery_required=true`n" +
        "phase=$($Transaction.Phase)`n"
    )
    } finally { $control.Dispose() }
}

function Test-TargetHealthy {
    param([string]$Target)
    if ($Target -eq 'python') {
        return -not [string]::IsNullOrWhiteSpace((Get-PythonIdentity))
    }
    $process = Get-VerifiedRustProcess
    if ($null -eq $process) { return $false }
    return (Get-VerifiedRustHeartbeatState -Process $process) -eq 'healthy'
}

function Restore-TransactionSource {
    param($Transaction)
    Enter-CutoverMaintenance -Transaction $Transaction
    Set-CutoverPhase -Transaction $Transaction -Phase 'recovery_starting'
    if ($Transaction.SourceRuntime -eq 'python') {
        Stop-Rust
        Publish-Mode -Value 'python'
        Set-CutoverPhase -Transaction $Transaction -Phase 'recovery_source_starting'
        Exit-CutoverMaintenance -Transaction $Transaction
        Start-Python
        Bind-CutoverTarget $Transaction 'python'
    } else {
        Assert-RustCutoverPreflight
        Stop-Python
        Publish-Mode -Value 'rust'
        Set-CutoverPhase -Transaction $Transaction -Phase 'recovery_source_starting'
        Exit-CutoverMaintenance -Transaction $Transaction
        Start-Rust
        Bind-CutoverTarget $Transaction 'rust'
        Wait-RustHealthy
    }
    Set-CutoverPhase -Transaction $Transaction -Phase 'recovery_complete'
    Complete-CutoverState -Transaction $Transaction
}

function Recover-InterruptedCutover {
    $transaction = Get-CutoverState
    if ($null -eq $transaction) { return }
    if ($DryRun) {
        if ($Runtime -eq $transaction.SourceRuntime) {
            Write-Output (
                "would_explicitly_recover_interrupted_cutover " +
                "transaction=$($transaction.TransactionId) " +
                "source=$($transaction.SourceRuntime) phase=$($transaction.Phase)"
            )
        } else {
            Write-Output (
                "would_require_explicit_cutover_recovery " +
                "transaction=$($transaction.TransactionId) " +
                "source=$($transaction.SourceRuntime) phase=$($transaction.Phase) " +
                "rerun_runtime=$($transaction.SourceRuntime)"
            )
        }
        $script:CutoverRecoveryPreviewed = $true
        return
    }
    if (
        $transaction.OwnerIdentity -ne $CutoverIdentity -and
        (Test-ProcessIdentityAlive -Identity $transaction.OwnerIdentity)
    ) {
        throw "Another cutover process is active: identity=$($transaction.OwnerIdentity)"
    }
    if (
        $transaction.Phase -eq 'target_healthy' -and
        (Test-TargetHealthy -Target $(if ($transaction.CompletionRuntime) { $transaction.CompletionRuntime } else { $transaction.TargetRuntime }))
    ) {
        Complete-CutoverState -Transaction $transaction
        $completedRuntime = if ($transaction.CompletionRuntime) { $transaction.CompletionRuntime } else { $transaction.TargetRuntime }
        Write-Output "interrupted_cutover_finalized runtime=$completedRuntime"
        $script:CutoverRecoveryHandled = $true
        return
    }
    if ($Runtime -ne $transaction.SourceRuntime) {
        $reason = (
            "Interrupted cutover requires explicit source recovery; " +
            "automatic rollback is disabled. Run: " +
            ".\codex-discord-runtime-cutover.ps1 -Runtime $($transaction.SourceRuntime)"
        )
        Throw-CutoverFailure -Transaction $transaction -Failure $reason
    }
    try {
        Restore-TransactionSource -Transaction $transaction
        Write-Output (
            "interrupted_cutover_recovered runtime=$($transaction.SourceRuntime) " +
            "recovery=explicit"
        )
        $script:CutoverRecoveryHandled = $true
    } catch {
        Throw-CutoverFailure -Transaction $transaction -Failure $_
    }
}

function New-CutoverTransaction {
    param([string]$Source, [string]$Target)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
    Assert-CdrNoPendingRestart -Root $RepoRoot
    if (Test-Path -LiteralPath $CutoverStatePath) { throw 'Existing cutover state preserved' }
    if (Test-Path -LiteralPath $DisablePath) { throw 'Existing maintenance intent preserved' }
    $transaction = [pscustomobject]@{
        TransactionId = [guid]::NewGuid().ToString('N')
        OwnerIdentity = $CutoverIdentity
        SourceRuntime = $Source
        TargetRuntime = $Target
        Phase = 'created'
    }
    Set-CutoverPhase -Transaction $transaction -Phase 'created'
    return $transaction
    } finally { $control.Dispose() }
}

Recover-InterruptedCutover
if ($CutoverRecoveryHandled) {
    exit 0
}
if ($CutoverRecoveryPreviewed) {
    exit 0
}
if ($Runtime -eq 'rust') {
    Assert-RustCutoverPreflight
} else {
    Assert-PythonRollbackPreflight
}
if ($DryRun) {
    Write-Output "cutover_dry_run target=$Runtime python_running=$(-not [string]::IsNullOrWhiteSpace((Get-PythonIdentity))) rust_running=$($null -ne (Get-VerifiedRustProcess))"
    if ($Runtime -eq 'rust') {
        Write-Output 'would_create_verified_online_store_backup'
    } else {
        Write-Output "would_create_verified_python_store_backup path=$PythonBackupDatabase"
    }
    Write-Output "would_switch_runtime target=$Runtime observe_seconds=$ObserveSeconds"
    exit 0
}

if (Test-Path -LiteralPath $DisablePath -PathType Leaf) {
    throw "Runtime is disabled by an operator-owned marker: $DisablePath"
}
if ($Runtime -eq 'rust') {
    Backup-Store
} else {
    Backup-StoreWithPython
}
$sourceRuntime = Get-CurrentRuntime
$transaction = New-CutoverTransaction -Source $sourceRuntime -Target $Runtime
try {
    Enter-CutoverMaintenance -Transaction $transaction
    Set-CutoverPhase -Transaction $transaction -Phase 'source_stopping'
    if ($sourceRuntime -eq 'python') { Stop-Python } else { Stop-Rust }
    Set-CutoverPhase -Transaction $transaction -Phase 'source_stopped'
    Publish-Mode -Value $Runtime
    Set-CutoverPhase -Transaction $transaction -Phase 'target_starting'
    Exit-CutoverMaintenance -Transaction $transaction
    if ($Runtime -eq 'rust') {
        Start-Rust
        Bind-CutoverTarget $transaction 'rust'
        Set-CutoverPhase -Transaction $transaction -Phase 'target_started'
        Wait-RustHealthy
    } else {
        Start-Python
        Bind-CutoverTarget $transaction 'python'
        Set-CutoverPhase -Transaction $transaction -Phase 'target_started'
    }
    Complete-CutoverState -Transaction $transaction
} catch {
    # Preserve the primary error; recovery publication may itself be forbidden.
    # Do not repeat publication from finally after a guard has already rejected it.
    Throw-CutoverFailure -Transaction $transaction -Failure $_
}

if ($Runtime -eq 'rust') {
    Write-Output "cutover_complete runtime=rust observe_seconds=$ObserveSeconds"
} else {
    Write-Output 'cutover_complete runtime=python manual_rollback=verified'
}
