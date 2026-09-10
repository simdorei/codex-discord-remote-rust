[CmdletBinding()]
param(
    [string]$RepoRoot = $PSScriptRoot,
    [switch]$DryRun,
    [string]$BotId = '123456789012345678'
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = (Get-Location).Path
}

$SetupScript = Join-Path $RepoRoot 'setup_discord_bot.py'
$WatchdogScript = Join-Path $RepoRoot 'codex-discord-watchdog.ps1'
$WatchdogLauncher = Join-Path $RepoRoot 'codex-discord-watchdog-hidden.vbs'
$WatchdogTaskName = 'Codex Discord Bot'

function Resolve-PythonCommand {
    if (-not [string]::IsNullOrWhiteSpace($env:PYTHON_EXE)) {
        return @($env:PYTHON_EXE)
    }

    $portablePython = Join-Path $RepoRoot '.python-portable\python.exe'
    if (Test-Path -LiteralPath $portablePython) {
        return @($portablePython)
    }

    throw 'Portable Python 3.12 was not found. Run .\install.ps1 first to download it and pin PYTHON_EXE.'
}

function Register-DiscordWatchdogTask {
    if (-not (Test-Path -LiteralPath $WatchdogScript)) {
        throw "Discord watchdog was not found: $WatchdogScript"
    }
    if (-not (Test-Path -LiteralPath $WatchdogLauncher)) {
        throw "Hidden Discord watchdog launcher was not found: $WatchdogLauncher"
    }

    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    if ($DryRun) {
        Write-Output "Would register scheduled task '$WatchdogTaskName' for $identity every minute."
        return
    }

    $watchdogArguments = "//B //Nologo `"$WatchdogLauncher`""
    $action = New-ScheduledTaskAction `
        -Execute 'wscript.exe' `
        -Argument $watchdogArguments `
        -WorkingDirectory $RepoRoot
    $logonTrigger = New-ScheduledTaskTrigger -AtLogOn -User $identity
    $repeatTrigger = New-ScheduledTaskTrigger `
        -Once `
        -At (Get-Date).AddMinutes(1) `
        -RepetitionInterval (New-TimeSpan -Minutes 1) `
        -RepetitionDuration (New-TimeSpan -Days 3650)
    $settings = New-ScheduledTaskSettingsSet `
        -MultipleInstances IgnoreNew `
        -StartWhenAvailable `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit (New-TimeSpan -Minutes 30) `
        -RestartCount 3 `
        -RestartInterval (New-TimeSpan -Minutes 1)
    $principal = New-ScheduledTaskPrincipal `
        -UserId $identity `
        -LogonType Interactive `
        -RunLevel Highest
    $task = New-ScheduledTask `
        -Action $action `
        -Trigger @($logonTrigger, $repeatTrigger) `
        -Settings $settings `
        -Principal $principal `
        -Description 'Keeps Codex Discord Remote running with interactive administrator-window control.'

    Register-ScheduledTask -TaskName $WatchdogTaskName -InputObject $task -Force | Out-Null
    Enable-ScheduledTask -TaskName $WatchdogTaskName | Out-Null
    Start-ScheduledTask -TaskName $WatchdogTaskName
    Write-Output "Registered scheduled task: $WatchdogTaskName"
}

$command = @(Resolve-PythonCommand)
$exe = $command[0]
$baseArgs = @()
if ($command.Count -gt 1) {
    $baseArgs = $command[1..($command.Count - 1)]
}

$scriptArgs = @($SetupScript, '--repo-root', $RepoRoot, '--bot-id', $BotId)
if ($DryRun) {
    $scriptArgs += '--dry-run'
}

& $exe @baseArgs @scriptArgs
if ($LASTEXITCODE -ne 0) {
    throw "Discord bot setup failed with exit code ${LASTEXITCODE}: $exe $($baseArgs + $scriptArgs -join ' ')"
}

Register-DiscordWatchdogTask
