[CmdletBinding()]
param(
    [ValidateSet('rust')]
    [string]$Runtime = 'rust',
    [string]$RepoRoot,
    [string]$BinaryPath,
    [string]$EnvPath,
    [int]$ObserveSeconds = 30,
    [int]$HeartbeatMaxAgeSeconds = 45,
    [int]$HeartbeatBootstrapGraceSeconds = 120,
    [switch]$Recover,
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
$RustLock = Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'
$RustStop = Join-Path $RepoRoot '.codex_discord_rust.stop'
$RustRestart = Join-Path $RepoRoot '.codex_discord_rust.restart'
$RustHeartbeat = Join-Path $RepoRoot '.codex_discord_rust.heartbeat'
$RustWatchdog = Join-Path $RepoRoot 'codex-discord-rust-watchdog.ps1'
$DisablePath = Join-Path $RepoRoot '.codex_discord_bot.disabled'
. (Join-Path $RepoRoot 'codex-discord-rust-control.ps1')
. (Join-Path $PSScriptRoot 'scripts/CdrCutoverState.ps1')
. (Join-Path $PSScriptRoot 'scripts/CdrCutoverRuntime.ps1')
. (Join-Path $PSScriptRoot 'scripts/CdrCutoverCompletion.ps1')
. (Join-Path $PSScriptRoot 'scripts/CdrCutoverRecovery.ps1')
$CutoverRecoveryHandled = $false
$CutoverRecoveryPreviewed = $false


$self = [Diagnostics.Process]::GetCurrentProcess()
try {
    $selfTicks = $self.StartTime.ToUniversalTime().Ticks
    $selfTicks -= ($selfTicks % 10)
    $CutoverIdentity = "$PID|$selfTicks"
} finally {
    $self.Dispose()
}

Recover-InterruptedCutover
if ($CutoverRecoveryHandled -or $CutoverRecoveryPreviewed) { exit 0 }
if ($Recover) { throw 'No interrupted cutover exists; recovery was not started.' }
Assert-RustCutoverPreflight
if ($DryRun) {
    Write-Output "cutover_dry_run target=$Runtime rust_running=$($null -ne (Get-VerifiedRustProcess))"
    Write-Output 'would_create_verified_online_store_backup'
    Write-Output "would_switch_runtime target=$Runtime observe_seconds=$ObserveSeconds"
    exit 0
}
if (Test-Path -LiteralPath $DisablePath -PathType Leaf) {
    throw "Runtime is disabled by an operator-owned marker: $DisablePath"
}
$sourceRuntime = Get-CurrentRuntime
Backup-Store
$transaction = New-CutoverTransaction -Source $sourceRuntime -Target $Runtime
try {
    Enter-CutoverMaintenance -Transaction $transaction
    Set-CutoverPhase -Transaction $transaction -Phase 'source_stopping'
    Stop-Rust
    Set-CutoverPhase -Transaction $transaction -Phase 'source_stopped'
    Publish-Mode -Value $Runtime
    Set-CutoverPhase -Transaction $transaction -Phase 'target_starting'
    Exit-CutoverMaintenance -Transaction $transaction
    Start-Rust
    Bind-CutoverTarget $transaction 'rust'
    Set-CutoverPhase -Transaction $transaction -Phase 'target_started'
    Wait-RustHealthy
    Complete-CutoverState -Transaction $transaction
} catch {
    # Do not overwrite the original failure or a competing operator's intent.
    Throw-CutoverFailure -Transaction $transaction -Failure $_
}
Write-Output "cutover_complete runtime=rust observe_seconds=$ObserveSeconds"
