[CmdletBinding()]
param(
    [string]$RepoRoot
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Join-Path $PSScriptRoot '..\..\..'
}

$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
$RuntimeModePath = Join-Path $RepoRoot '.codex_discord_runtime'
$RuntimeMode = [string]$env:CODEX_DISCORD_RUNTIME
if ([string]::IsNullOrWhiteSpace($RuntimeMode) -and (Test-Path -LiteralPath $RuntimeModePath)) {
    $RuntimeMode = (Get-Content -LiteralPath $RuntimeModePath -Raw).Trim()
}
if ([string]::IsNullOrWhiteSpace($RuntimeMode)) {
    $RuntimeMode = 'rust'
}
if ($RuntimeMode -eq 'rust') {
    & (Join-Path $RepoRoot 'codex-discord-rust-status.ps1') -RepoRoot $RepoRoot
    exit $LASTEXITCODE
}
if ($RuntimeMode -ne 'python') {
    throw (
        "Unsupported Codex Discord runtime selection: '$RuntimeMode'. " +
        "Expected 'rust' or an explicit manual 'python' rollback selection."
    )
}
Write-Warning 'Explicit manual Python rollback runtime is starting.'
. (Join-Path $RepoRoot 'codex-discord-python-runtime.ps1')
$RuntimeLock = Join-Path $RepoRoot '.codex_discord_bot.runtime.lock'
$RestartMarker = Join-Path $RepoRoot '.codex_discord_bot.restart'
$StopMarker = Join-Path $RepoRoot '.codex_discord_bot.stop'
$LogPath = Join-Path $RepoRoot 'codex_discord_bot.log'
$LauncherLogPath = Join-Path $RepoRoot 'discord_launcher.log'
$BridgePath = Join-Path $RepoRoot 'codex_desktop_bridge.py'
$CodexHome = $env:CODEX_HOME
if ([string]::IsNullOrWhiteSpace($CodexHome)) {
    $CodexHome = Join-Path $HOME '.codex'
}
$BridgeStatePath = $env:CODEX_BRIDGE_STATE
if ([string]::IsNullOrWhiteSpace($BridgeStatePath)) {
    $BridgeStatePath = Join-Path $CodexHome 'codex_desktop_bridge_state.json'
}

function Format-OptionalValue {
    param([string]$Value)

    if ([string]::IsNullOrWhiteSpace($Value)) {
        return '-'
    }
    return $Value
}

function Get-CodexRuntimePythonExecutable {
    return Resolve-CodexRuntimePythonExecutable -RepoRoot $RepoRoot
}

function Write-CodexAppPackageUpdateStatus {
    $currentVersion = $null
    $versionStatus = 'Get-AppxPackage OpenAI.Codex'
    try {
        $package = Get-AppxPackage -Name OpenAI.Codex -ErrorAction Stop | Select-Object -First 1
        if ($null -ne $package -and $null -ne $package.Version) {
            $currentVersion = [string]$package.Version
        }
    } catch {
        $versionStatus = $_.Exception.Message
    }

    $previousVersion = $null
    $updateDetected = $false
    if (-not [string]::IsNullOrWhiteSpace($currentVersion)) {
        $state = [ordered]@{}
        $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
        if (Test-Path -LiteralPath $BridgeStatePath) {
            try {
                $loaded = [System.IO.File]::ReadAllText($BridgeStatePath, $utf8NoBom) | ConvertFrom-Json
                if ($loaded) {
                    foreach ($property in $loaded.PSObject.Properties) {
                        $state[$property.Name] = $property.Value
                    }
                }
            } catch {
                Write-Output "codex_app_package_version_state_error: $($_.Exception.Message)"
                $state = $null
            }
        }
        if ($null -ne $state) {
            if ($state.Contains('codex_app_package_version')) {
                $previousVersion = [string]$state['codex_app_package_version']
            }
            $updateDetected = -not [string]::IsNullOrWhiteSpace($previousVersion) -and $previousVersion -ne $currentVersion
            $state['codex_app_package_version'] = $currentVersion
            $stateDir = Split-Path -Parent $BridgeStatePath
            if (-not [string]::IsNullOrWhiteSpace($stateDir)) {
                New-Item -ItemType Directory -Force -Path $stateDir | Out-Null
            }
            $json = $state | ConvertTo-Json -Depth 20
            [System.IO.File]::WriteAllText($BridgeStatePath, $json + [Environment]::NewLine, $utf8NoBom)
        }
    }

    Write-Output "codex_app_package_version: $(Format-OptionalValue $currentVersion)"
    Write-Output "codex_app_previous_package_version: $(Format-OptionalValue $previousVersion)"
    Write-Output "codex_app_update_detected: $updateDetected"
    Write-Output "codex_app_restart_recommended: $updateDetected"
    Write-Output "codex_app_package_version_status: $versionStatus"
}

Write-Output "repo: $RepoRoot"

if (Test-Path -LiteralPath (Join-Path $RepoRoot '.git')) {
    git -C $RepoRoot status --short --branch
}

if (Test-Path -LiteralPath $RuntimeLock) {
    $pidText = (Get-Content -LiteralPath $RuntimeLock -Raw).Trim()
    Write-Output "runtime_lock_pid: $pidText"
    if ($pidText -match '^\d+$') {
        $process = Get-Process -Id ([int]$pidText) -ErrorAction SilentlyContinue
        if ($process) {
            Write-Output "bot_process: running pid=$($process.Id) name=$($process.ProcessName) started=$($process.StartTime)"
        } else {
            Write-Output 'bot_process: not found'
        }
    }
} else {
    Write-Output 'runtime_lock_pid: missing'
}

if (Test-Path -LiteralPath $RestartMarker) {
    Write-Output "restart_marker: present path=$RestartMarker"
} else {
    Write-Output 'restart_marker: missing'
}

if (Test-Path -LiteralPath $StopMarker) {
    Write-Output "stop_marker: present path=$StopMarker"
} else {
    Write-Output 'stop_marker: missing'
}

Write-CodexAppPackageUpdateStatus

if (Test-Path -LiteralPath $LogPath) {
    Write-Output ''
    Write-Output 'recent_log:'
    Get-Content -LiteralPath $LogPath -Tail 40
} else {
    Write-Output 'recent_log: missing'
}

if (Test-Path -LiteralPath $LauncherLogPath) {
    Write-Output ''
    Write-Output 'launcher_log:'
    Get-Content -LiteralPath $LauncherLogPath -Tail 20
} else {
    Write-Output 'launcher_log: missing'
}

if (Test-Path -LiteralPath $BridgePath) {
    Write-Output ''
    Write-Output 'bridge_threads:'
    $pythonExecutable = Get-CodexRuntimePythonExecutable
    & $pythonExecutable $BridgePath list --db-root --limit 8
}
