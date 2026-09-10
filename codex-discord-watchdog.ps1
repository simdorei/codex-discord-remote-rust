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
if ($RuntimeMode -ne 'python') {
    throw (
        "Unsupported Codex Discord runtime selection: '$RuntimeMode'. " +
        "Expected 'rust' or an explicit manual 'python' rollback selection."
    )
}
Write-Warning 'Explicit manual Python rollback runtime is starting.'
$BotScript = Join-Path $ScriptDir 'codex_discord_bot.py'
$BridgePath = Join-Path $ScriptDir 'codex_desktop_bridge.py'
$RuntimeLockPath = Join-Path $ScriptDir '.codex_discord_bot.runtime.lock'
$RestartRequestPath = Join-Path $ScriptDir '.codex_discord_bot.restart'
$WatchdogProcess = [Diagnostics.Process]::GetCurrentProcess()
try {
    $WatchdogStartedTicks = $WatchdogProcess.StartTime.ToUniversalTime().Ticks
    $WatchdogStartedTicks -= ($WatchdogStartedTicks % 10)
} finally {
    $WatchdogProcess.Dispose()
}
$RestartClaimPath = Join-Path $ScriptDir ".codex_discord_bot.restart.claimed.$PID.$WatchdogStartedTicks"
$RestartClaimPattern = Join-Path $ScriptDir '.codex_discord_bot.restart.claimed.*'
$ResumeRemoteMcpContext = $false
$HealthStatePath = Join-Path $ScriptDir '.codex_discord_bot.health'
$HeartbeatPath = Join-Path $ScriptDir '.codex_discord_bot.heartbeat'
$StopRequestPath = Join-Path $ScriptDir '.codex_discord_bot.stop'
$StopClaimPath = Join-Path $ScriptDir ".codex_discord_bot.stop.claimed.$PID.$WatchdogStartedTicks"
$StopClaimPattern = Join-Path $ScriptDir '.codex_discord_bot.stop.claimed.*'
$DisablePath = Join-Path $ScriptDir '.codex_discord_bot.disabled'
$HeadlessLauncher = Join-Path $ScriptDir 'codex-discord-bot-headless.vbs'
$LauncherLogPath = Join-Path $ScriptDir 'discord_launcher.log'
. (Join-Path $ScriptDir 'codex-discord-atomic-file-runtime.ps1')
. (Join-Path $ScriptDir 'codex-discord-watchdog-runtime.ps1')
. (Join-Path $ScriptDir 'codex-discord-watchdog-restart-runtime.ps1')
. (Join-Path $ScriptDir 'codex-discord-watchdog-heartbeat-runtime.ps1')
. (Join-Path $ScriptDir 'codex-discord-watchdog-identity-runtime.ps1')
. (Join-Path $ScriptDir 'codex-discord-watchdog-stop-runtime.ps1')

function Test-ChatGptDesktopProcessAlive {
    $desktopProcess = Get-Process -Name 'ChatGPT' -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Path -like '*\WindowsApps\OpenAI.Codex_*\app\ChatGPT.exe'
        } |
        Select-Object -First 1
    return $null -ne $desktopProcess
}

function Ensure-ChatGptDesktopRunning {
    param([switch]$DryRun)

    if (Test-ChatGptDesktopProcessAlive) {
        return
    }

    $package = Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction Stop |
        Select-Object -First 1
    if ($null -eq $package) {
        throw 'ChatGPT AppX package OpenAI.Codex is not installed.'
    }

    $manifest = Get-AppxPackageManifest -Package $package
    $application = @($manifest.Package.Applications.Application) |
        Where-Object { $_.Id -eq 'App' } |
        Select-Object -First 1
    if ($null -eq $application) {
        throw 'ChatGPT AppX application id App is not registered.'
    }

    $appUserModelId = "$($package.PackageFamilyName)!$($application.Id)"
    if ($DryRun) {
        Write-Output "chatgpt_would_start app_id=$appUserModelId"
        return
    }

    Start-Process -FilePath 'explorer.exe' -ArgumentList @("shell:AppsFolder\$appUserModelId")
    Write-LauncherLog "watchdog_chatgpt_start app_id=$appUserModelId"
}

if (-not (Test-Path -LiteralPath $BotScript)) {
    Write-LauncherLog "watchdog_error reason=bot_script_missing script=$BotScript"
    exit 1
}

function Get-WatchdogSystemHealthIssue {
    try {
        $cpuAverage = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
        $performanceCpuAverage = $null
        if (
            $HealthCpuPercent -gt 0 -and
            $cpuAverage -ne $null -and
            [double]$cpuAverage -ge $HealthCpuPercent
        ) {
            $performanceCpuAverage = (
                (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples 5 -ErrorAction Stop).CounterSamples |
                Measure-Object -Property CookedValue -Average
            ).Average
        }
        $os = Get-CimInstance Win32_OperatingSystem
        $freeMemoryMb = [math]::Floor([double]$os.FreePhysicalMemory / 1024)
    } catch {
        Write-LauncherLog "watchdog_health_probe_failed error=$($_.Exception.GetType().Name)"
        return ""
    }

    $issues = @()
    if ($HealthCpuPercent -gt 0 -and $cpuAverage -ne $null) {
        $wmiCpuPercent = [math]::Round([double]$cpuAverage, 1)
        if ($wmiCpuPercent -ge $HealthCpuPercent -and $performanceCpuAverage -ne $null) {
            $cpuPercent = [math]::Round([double]$performanceCpuAverage, 1)
            if ($cpuPercent -ge $HealthCpuPercent) {
                $issues += "cpu_percent=$cpuPercent threshold=$HealthCpuPercent"
            }
        }
    }
    if ($HealthFreeMemoryMb -gt 0 -and $freeMemoryMb -le $HealthFreeMemoryMb) {
        $issues += "free_memory_mb=$freeMemoryMb threshold=$HealthFreeMemoryMb"
    }
    $heartbeatIssue = Get-WatchdogHeartbeatIssue `
        -HeartbeatPath $HeartbeatPath `
        -MaxAgeSeconds $HealthHeartbeatMaxAgeSeconds `
        -StartupGraceSeconds $HealthHeartbeatStartupGraceSeconds `
        -ProcessAgeSeconds (Get-BotProcessAgeSeconds)
    if ($heartbeatIssue) {
        $issues += $heartbeatIssue
    }
    if ($issues.Count -eq 0) {
        return ""
    }
    return ($issues -join ",")
}

function Get-WatchdogHealthBadSampleCount {
    if (-not (Test-Path -LiteralPath $HealthStatePath)) {
        return 0
    }
    try {
        $stateText = Get-Content -LiteralPath $HealthStatePath -Raw -ErrorAction Stop
        if ($stateText -match 'count=(\d+)') {
            return [int]$Matches[1]
        }
    } catch {
        Write-LauncherLog "watchdog_health_state_read_failed error=$($_.Exception.GetType().Name)"
    }
    return 0
}

function Set-WatchdogHealthBadSampleCount {
    param(
        [int]$Count,
        [string]$Issue
    )

    Set-Content -LiteralPath $HealthStatePath -Encoding ASCII -Value "count=$Count issue=$Issue"
}

function Clear-WatchdogHealthState {
    Remove-Item -LiteralPath $HealthStatePath -Force -ErrorAction SilentlyContinue
}

function Get-WatchdogHealthRestartIssue {
    $issue = Get-WatchdogSystemHealthIssue
    if (-not $issue) {
        Clear-WatchdogHealthState
        return ""
    }

    $sampleCount = (Get-WatchdogHealthBadSampleCount) + 1
    $sampleLimit = [math]::Max(1, $HealthBadSampleLimit)
    Set-WatchdogHealthBadSampleCount -Count $sampleCount -Issue $issue
    Write-LauncherLog "watchdog_unhealthy_sample count=$sampleCount limit=$sampleLimit $issue"
    if ($sampleCount -lt $sampleLimit) {
        return ""
    }
    return $issue
}

if ($CheckRestartReady) {
    Wait-CodexThreadsQuietForRestart
    Write-Output "restart_check_ok"
    exit 0
}

if (-not $DryRun) {
    Restore-OrphanedStopClaims
}

if (Test-Path -LiteralPath $StopRequestPath) {
    if ($DryRun) {
        Write-Output "stop_requested"
        exit 0
    }
    $claimedStopPath = ""
    try {
        Move-Item `
            -LiteralPath $StopRequestPath `
            -Destination $StopClaimPath `
            -ErrorAction Stop
        $claimedStopPath = $StopClaimPath
    } catch {
        if (-not (Test-Path -LiteralPath $StopRequestPath)) {
            Write-LauncherLog "watchdog_stop_claim_lost marker=$StopRequestPath"
            exit 0
        }
        throw
    }
    $stopIdentity = Get-BoundProcessIdentityFromMarker -MarkerPath $claimedStopPath
    if (-not $stopIdentity) {
        Write-LauncherLog "watchdog_stop_stale_unbound marker=$claimedStopPath"
        Remove-Item -LiteralPath $claimedStopPath -Force -ErrorAction SilentlyContinue
    } else {
        $stopPid = [int]($stopIdentity -split '\|', 2)[0]
        $currentStopIdentity = Get-CodexBotProcessIdentityById `
            -BotScript $BotScript `
            -ProcessId $stopPid
        if ($currentStopIdentity -ne $stopIdentity) {
            Write-LauncherLog (
                "watchdog_stop_stale marker=$claimedStopPath " +
                "expected=$stopIdentity current=$currentStopIdentity"
            )
            Remove-Item -LiteralPath $claimedStopPath -Force -ErrorAction SilentlyContinue
        } else {
            try {
                Write-LauncherLog "watchdog_stop_requested marker=$claimedStopPath identity=$stopIdentity"
                Stop-RuntimeBotProcess -ExpectedIdentity $stopIdentity
            } finally {
                Remove-Item -LiteralPath $claimedStopPath -Force -ErrorAction SilentlyContinue
            }
            exit 0
        }
    }
}

if (Test-Path -LiteralPath $DisablePath) {
    if ($DryRun) {
        Write-Output "disabled"
    } elseif ($LogHealthy) {
        Write-LauncherLog "watchdog_disabled marker=$DisablePath"
    }
    exit 0
}

Ensure-ChatGptDesktopRunning -DryRun:$DryRun

if (-not $DryRun) {
    Restore-OrphanedRestartClaims
}

if (Test-Path -LiteralPath $RestartRequestPath) {
    if ($DryRun) {
        Write-Output "restart_requested"
        exit 0
    }
    $claimedRestartPath = Claim-RestartRequest
    if (-not $claimedRestartPath) {
        if (Test-BotProcessAlive) {
            exit 0
        }
    } elseif (-not (Test-RestartClaimMatchesCurrentBot `
        -ClaimPath $claimedRestartPath `
        -BotScript $BotScript `
        -RuntimeLockPath $RuntimeLockPath)) {
        Write-LauncherLog "watchdog_restart_stale claim=$claimedRestartPath"
        Remove-Item -LiteralPath $claimedRestartPath -Force -ErrorAction SilentlyContinue
        exit 0
    }
    try {
        Wait-CodexThreadsQuietForRestart
    } catch {
        Restore-RestartRequest -ClaimPath $claimedRestartPath
        Write-LauncherLog "watchdog_restart_refused error=$($_.Exception.Message)"
        throw
    }
    if ($claimedRestartPath -and -not (Test-RestartClaimMatchesCurrentBot `
        -ClaimPath $claimedRestartPath `
        -BotScript $BotScript `
        -RuntimeLockPath $RuntimeLockPath)) {
        Write-LauncherLog "watchdog_restart_identity_changed claim=$claimedRestartPath"
        Remove-Item -LiteralPath $claimedRestartPath -Force -ErrorAction SilentlyContinue
        exit 0
    }
    try {
        Write-LauncherLog "watchdog_restart_requested marker=$claimedRestartPath"
        $claimedIdentity = Get-BoundProcessIdentityFromMarker `
            -MarkerPath $claimedRestartPath
        Stop-RuntimeBotProcess `
            -ExpectedIdentity $claimedIdentity `
            -PreserveRemoteMcpContext
        $ResumeRemoteMcpContext = $true
    } finally {
        Remove-Item -LiteralPath $claimedRestartPath -Force -ErrorAction SilentlyContinue
    }
}

if (Test-BotProcessAlive) {
    $healthRestartIssue = ""
    if (-not $DryRun) {
        $healthRestartIssue = Get-WatchdogHealthRestartIssue
    }
    if ($healthRestartIssue) {
        Write-LauncherLog "watchdog_restart_unhealthy reason=$healthRestartIssue"
        $healthRestartIdentity = Get-CodexBotProcessIdentity `
            -BotScript $BotScript `
            -RuntimeLockPath $RuntimeLockPath
        if (-not $healthRestartIdentity) {
            throw 'Unhealthy bot identity changed before restart could stop it.'
        }
        Stop-RuntimeBotProcess -ExpectedIdentity $healthRestartIdentity
        Clear-WatchdogHealthState
    }
    if (Test-BotProcessAlive) {
        if ($LogHealthy) {
            Write-LauncherLog "watchdog_ok script=$BotScript"
        }
        if ($DryRun) {
            Write-Output "running"
        }
        exit 0
    }
}

if (-not (Test-Path -LiteralPath $HeadlessLauncher)) {
    Write-LauncherLog "watchdog_error reason=headless_launcher_missing launcher=$HeadlessLauncher"
    exit 1
}

if ($DryRun) {
    Write-Output "would_start"
    exit 0
}

Write-LauncherLog "watchdog_start_missing script=$BotScript launcher=$HeadlessLauncher"
if ($ResumeRemoteMcpContext) {
    $env:CODEX_REMOTE_MCP_RESTART_RESUME = '1'
} else {
    Remove-Item Env:CODEX_REMOTE_MCP_RESTART_RESUME -ErrorAction SilentlyContinue
}
Start-Process -FilePath 'wscript.exe' -ArgumentList @("`"$HeadlessLauncher`"") -WindowStyle Hidden
exit 0
