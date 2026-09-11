[CmdletBinding()]
param(
    [switch]$Once
)

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$TrayRuntimePath = Join-Path $ScriptDir 'codex-discord-tray-runtime.ps1'
$TrayRestartRuntimePath = Join-Path $ScriptDir 'codex-discord-tray-restart-runtime.ps1'
$HeadlessLauncher = Join-Path $ScriptDir 'codex-discord-bot-headless.vbs'
$LauncherLogPath = Join-Path $ScriptDir 'discord_launcher.log'
$StoppedGraceSeconds = 30
$script:MissingSince = $null
$script:LastRunningStatus = $null
$script:LastTrayIcon = $null

foreach ($runtimePath in @(
    $TrayRuntimePath,
    $TrayRestartRuntimePath
)) {
    if (-not (Test-Path -LiteralPath $runtimePath)) {
        throw "tray runtime script not found: $runtimePath"
    }
}
. $TrayRuntimePath
$RuntimeMode = Resolve-TrayRuntimeMode
$BotLogPath = Join-Path $ScriptDir 'codex_discord_rust.log'
. $TrayRestartRuntimePath

function Write-LauncherLog {
    param([string]$Message)

    $timestamp = (Get-Date).ToString('s')
    Add-Content -LiteralPath $LauncherLogPath -Encoding UTF8 -Value "[$timestamp] $Message"
}

function Get-BotStatus {
    $process = Get-BotProcess
    if ($process -eq $null) {
        return [pscustomobject]@{
            Running = $false
            Pid = $null
            Text = 'Codex Discord bridge stopped'
            Icon = 'Warning'
        }
    }
    return [pscustomobject]@{
        Running = $true
        Pid = [int]$process.ProcessId
        Text = "Codex Discord bridge running (PID $($process.ProcessId))"
        Icon = 'Application'
    }
}

function Limit-TrayText {
    param([string]$Text)

    if ($Text.Length -le 63) {
        return $Text
    }
    return $Text.Substring(0, 60) + '...'
}

if ($Once) {
    $status = Get-BotStatus
    if ($status.Running) {
        Write-Output "running pid=$($status.Pid)"
        exit 0
    }
    Write-Output "stopped"
    exit 1
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$mutexHash = [BitConverter]::ToString(
    [Security.Cryptography.SHA256]::Create().ComputeHash(
        [Text.Encoding]::UTF8.GetBytes([IO.Path]::GetFullPath($ScriptDir).ToLowerInvariant())
    )
).Replace('-', '').Substring(0, 16)
$createdNew = $false
$mutex = New-Object Threading.Mutex($true, "Local\CodexDiscordTray_$mutexHash", [ref]$createdNew)
if (-not $createdNew) {
    Write-LauncherLog "tray_duplicate_exit script=$PSCommandPath"
    exit 0
}

$notify = New-Object Windows.Forms.NotifyIcon
$notify.Icon = [Drawing.SystemIcons]::Information
$notify.Text = 'Codex Discord bridge starting'
$notify.Visible = $true

$menu = New-Object Windows.Forms.ContextMenuStrip
$statusItem = New-Object Windows.Forms.ToolStripMenuItem
$statusItem.Text = 'Starting...'
$statusItem.Enabled = $false
[void]$menu.Items.Add($statusItem)

$openLogItem = New-Object Windows.Forms.ToolStripMenuItem
$openLogItem.Text = 'Open bot log'
$openLogItem.Add_Click({
    if (Test-Path -LiteralPath $BotLogPath) {
        Start-Process -FilePath 'notepad.exe' -ArgumentList @("`"$BotLogPath`"")
    }
})
[void]$menu.Items.Add($openLogItem)

$openFolderItem = New-Object Windows.Forms.ToolStripMenuItem
$openFolderItem.Text = 'Open bridge folder'
$openFolderItem.Add_Click({
    Start-Process -FilePath 'explorer.exe' -ArgumentList @("`"$ScriptDir`"")
})
[void]$menu.Items.Add($openFolderItem)

$restartItem = New-Object Windows.Forms.ToolStripMenuItem
$restartItem.Text = 'Restart bot'
$restartItem.Add_Click({
    try {
        Request-BotRestart
        $notify.ShowBalloonTip(3000, 'Codex Discord bridge', 'Restart requested.', [Windows.Forms.ToolTipIcon]::Info)
    } catch {
        Write-LauncherLog "tray_restart_failed error=$($_.Exception.Message)"
        $notify.ShowBalloonTip(3000, 'Codex Discord bridge', 'Restart request failed. Check discord_launcher.log.', [Windows.Forms.ToolTipIcon]::Error)
    }
})
[void]$menu.Items.Add($restartItem)

[void]$menu.Items.Add((New-Object Windows.Forms.ToolStripSeparator))

$exitItem = New-Object Windows.Forms.ToolStripMenuItem
$exitItem.Text = 'Exit tray icon'
$exitItem.Add_Click({
    [Windows.Forms.Application]::Exit()
})
[void]$menu.Items.Add($exitItem)

$notify.ContextMenuStrip = $menu

function Update-TrayStatus {
    $status = Get-BotStatus

    if ($status.Running) {
        $script:MissingSince = $null
        $script:LastRunningStatus = $status
    } else {
        if ($script:MissingSince -eq $null) {
            $script:MissingSince = Get-Date
        }
        $missingSeconds = ((Get-Date) - $script:MissingSince).TotalSeconds
        if ($script:LastRunningStatus -ne $null -and $missingSeconds -lt $StoppedGraceSeconds) {
            $status = [pscustomobject]@{
                Running = $true
                Pid = $script:LastRunningStatus.Pid
                Text = "Codex Discord bridge restarting (last PID $($script:LastRunningStatus.Pid))"
                Icon = 'Application'
            }
        }
    }

    $statusItem.Text = $status.Text
    $notify.Text = Limit-TrayText $status.Text
    $nextIcon = if ($status.Running) { 'Application' } else { 'Warning' }
    if ($script:LastTrayIcon -ne $nextIcon) {
        if ($status.Running) {
            $notify.Icon = [Drawing.SystemIcons]::Application
        } else {
            $notify.Icon = [Drawing.SystemIcons]::Warning
        }
        $script:LastTrayIcon = $nextIcon
    }
}

$timer = New-Object Windows.Forms.Timer
$timer.Interval = 5000
$timer.Add_Tick({ Update-TrayStatus })

try {
    Write-LauncherLog "tray_start script=$PSCommandPath"
    [Windows.Forms.Application]::EnableVisualStyles()
    Update-TrayStatus
    $timer.Start()
    [Windows.Forms.Application]::Run()
} finally {
    $timer.Stop()
    $notify.Visible = $false
    $notify.Dispose()
    $mutex.ReleaseMutex()
    $mutex.Dispose()
    Write-LauncherLog "tray_exit script=$PSCommandPath"
}
