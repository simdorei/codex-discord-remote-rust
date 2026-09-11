[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$ExpectedPid,
    [Parameter(Mandatory)][long]$ExpectedStartTicks,
    [switch]$CheckOnly
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$binary = Join-Path $root 'target\release\cdr-runtime.exe'
$stop = Join-Path $root '.codex_discord_rust.stop'
$watchdog = Join-Path $root 'codex-discord-rust-watchdog.ps1'
. (Join-Path $root 'codex-discord-rust-drain.ps1')
Import-Module (Join-Path $PSScriptRoot 'CdrNativeProcess.psm1') -Force

function Assert-OriginalRuntime {
    $process = Get-Process -Id $ExpectedPid -ErrorAction Stop
    if ($process.Path -ne $binary -or
        $process.StartTime.ToUniversalTime().Ticks -ne $ExpectedStartTicks) {
        throw 'Runtime identity changed; no stop requested.'
    }
    return $process
}

function Get-ActiveQueueCount {
    $null = Assert-OriginalRuntime
    $result = Invoke-CdrNative -Executable $binary -Arguments @(
        '--admin', 'active-queue-count', '--repo-root', $root
    ) -TimeoutSeconds 10
    if ($null -eq $result -or "$result".Trim() -notmatch '^\d+$') {
        throw 'Cannot inspect active queue: invalid native count receipt; no stop requested.'
    }
    return [int]$result
}

Set-Location -LiteralPath $root
$original = Assert-OriginalRuntime
foreach ($name in @('.codex_discord_bot.disabled', '.codex_discord_rust.stop',
    '.codex_discord_rust.restart', '.codex_discord_rust.drain.prepare')) {
    if (Test-Path -LiteralPath (Join-Path $root $name)) {
        throw "Existing control marker prevents recovery: $name"
    }
}
if ($CheckOnly) {
    Write-Output "precheck_ok active_queue=$(Get-ActiveQueueCount) runtime=$ExpectedPid"
    exit 0
}
throw 'Queue-only restart is retired: use codex-discord-rust-restart.ps1 with full readiness and identity-bound completion. No stop requested.'
