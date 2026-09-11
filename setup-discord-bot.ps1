[CmdletBinding()]
param(
    [string]$RepoRoot,
    [switch]$DryRun,
    [string]$BotId = '123456789012345678',
    [string]$BinaryPath
)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) { $RepoRoot = $PSScriptRoot }
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
if (-not $BinaryPath) {
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    if (-not [IO.Path]::IsPathRooted($target)) { $target = Join-Path $RepoRoot $target }
    $BinaryPath = Join-Path $target 'release\cdr-runtime.exe'
}
if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) { throw 'Rust runtime was not found. Run install.ps1 first.' }
Import-Module (Join-Path $RepoRoot 'scripts\CdrNativeProcess.psm1') -Force
$WatchdogScript = Join-Path $RepoRoot 'codex-discord-watchdog.ps1'
$WatchdogLauncher = Join-Path $RepoRoot 'codex-discord-watchdog-hidden.vbs'
$WatchdogTaskName = 'Codex Discord Bot'

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


$arguments = @('--admin', 'setup-discord', '--repo-root', $RepoRoot)
if ($DryRun) {
    Invoke-CdrNative -Executable $BinaryPath -Arguments ($arguments + @('--dry-run', '--bot-id', $BotId))
} else {
    $secureToken = Read-Host 'Discord bot token (hidden)' -AsSecureString
    $tokenPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken)
    try {
        $tokenText = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($tokenPointer)
        $channel = Read-Host 'Discord general channel ID (optional)'
        $inputJson = @{ token = $tokenText; channel_id = $channel } | ConvertTo-Json -Compress
        Invoke-CdrNative -Executable $BinaryPath -Arguments ($arguments + @('--input-stdin')) -InputText $inputJson
    } finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($tokenPointer)
        $tokenText = $null
        $inputJson = $null
        $secureToken.Dispose()
    }
}
Register-DiscordWatchdogTask
