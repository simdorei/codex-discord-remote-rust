[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$LogHealthy,
    [switch]$CheckRestartReady,
    [int]$RestartQuietSeconds = 90,
    [int]$RestartWaitTimeoutSeconds = 900,
    [int]$HealthCpuPercent = 95,
    [int]$HealthFreeMemoryMb = 768,
    [int]$HealthHeartbeatMaxAgeSeconds = 45,
    [int]$HealthHeartbeatStartupGraceSeconds = 120,
    [int]$HealthBadSampleLimit = 2
)

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RuntimeModePath = Join-Path $ScriptDir '.codex_discord_runtime'
$RuntimeMode = [string]$env:CODEX_DISCORD_RUNTIME
if ([string]::IsNullOrWhiteSpace($RuntimeMode) -and (Test-Path -LiteralPath $RuntimeModePath)) {
    $RuntimeMode = (Get-Content -LiteralPath $RuntimeModePath -Raw).Trim()
}
if ([string]::IsNullOrWhiteSpace($RuntimeMode)) {
    $RuntimeMode = 'rust'
}
if ($RuntimeMode -eq 'rust') {
    $RustWatchdog = Join-Path $ScriptDir 'codex-discord-rust-watchdog.ps1'
    if (-not (Test-Path -LiteralPath $RustWatchdog)) {
        throw "Rust watchdog was not found: $RustWatchdog"
    }
    & $RustWatchdog `
        -RepoRoot $ScriptDir `
        -DryRun:$DryRun `
        -LogHealthy:$LogHealthy `
        -CheckRestartReady:$CheckRestartReady `
        -RestartQuietSeconds $RestartQuietSeconds `
        -RestartWaitTimeoutSeconds $RestartWaitTimeoutSeconds `
        -HealthCpuPercent $HealthCpuPercent `
        -HealthFreeMemoryMb $HealthFreeMemoryMb `
        -HealthHeartbeatMaxAgeSeconds $HealthHeartbeatMaxAgeSeconds `
        -HealthHeartbeatStartupGraceSeconds $HealthHeartbeatStartupGraceSeconds `
        -HealthBadSampleLimit $HealthBadSampleLimit
    exit $LASTEXITCODE
}
throw "This installation only supports the Rust runtime; unsupported selection: '$RuntimeMode'."
