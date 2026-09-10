function Resolve-TrayRuntimeMode {
    $mode = [string]$env:CODEX_DISCORD_RUNTIME
    $modePath = Join-Path $ScriptDir '.codex_discord_runtime'
    if ([string]::IsNullOrWhiteSpace($mode) -and (Test-Path -LiteralPath $modePath)) {
        $mode = (Get-Content -LiteralPath $modePath -Raw -Encoding UTF8).Trim()
    }
    if ([string]::IsNullOrWhiteSpace($mode)) { return 'rust' }
    if ($mode -eq 'rust' -or $mode -eq 'python') { return $mode.ToLowerInvariant() }
    throw "Unsupported Codex Discord runtime selection: '$mode'."
}

function Get-RustTrayProcess {
    $lockPath = Join-Path $ScriptDir '.codex_discord_rust.runtime.lock'
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) { return $null }
    $lockText = Get-Content -LiteralPath $lockPath -Raw -Encoding UTF8
    if ($lockText -notmatch '(?m)^pid=(\d+)\r?$') { return $null }
    $process = Get-Process -Id ([int]$Matches[1]) -ErrorAction SilentlyContinue
    if ($null -eq $process) { return $null }
    $actualPath = [string]$process.Path
    if ([string]::IsNullOrWhiteSpace($actualPath)) { return $null }
    $expectedPath = [IO.Path]::GetFullPath((Join-Path $ScriptDir 'target\release\cdr-runtime.exe'))
    if (-not [IO.Path]::GetFullPath($actualPath).Equals(
        $expectedPath, [StringComparison]::OrdinalIgnoreCase
    )) { return $null }
    return [pscustomobject]@{
        ProcessId = [int]$process.Id
        StartTime = $process.StartTime
    }
}

function Get-RustTrayProcessIdentity {
    param($Process)
    if ($null -eq $Process) { return '' }
    $ticks = $Process.StartTime.ToUniversalTime().Ticks
    $ticks -= ($ticks % 10)
    return "$([int]$Process.ProcessId)|$ticks"
}

function Test-IsBotProcess {
    param($Process, [switch]$AllowRuntimeLockFallback)
    if ($null -eq $Process) { return $false }
    $name = [string]$Process.Name
    if ($name -ne 'py.exe' -and $name -ne 'python.exe' -and $name -ne 'pythonw.exe') {
        return $false
    }
    $needle = [IO.Path]::GetFullPath($BotScript).ToLowerInvariant()
    $commandLine = ([string]$Process.CommandLine).ToLowerInvariant()
    if (-not $commandLine -and $AllowRuntimeLockFallback) { return $true }
    return $commandLine.Contains($needle)
}

function Get-BotProcess {
    if ($RuntimeMode -eq 'rust') { return Get-RustTrayProcess }
    if ($RuntimeMode -ne 'python') { throw 'Tray runtime mode was not initialized.' }
    if (Test-Path -LiteralPath $RuntimeLockPath) {
        $pidText = (Get-Content -LiteralPath $RuntimeLockPath -Raw -ErrorAction SilentlyContinue).Trim()
        if ($pidText -match '^\d+$') {
            $process = Get-CimInstance Win32_Process -Filter "ProcessId=$pidText" -ErrorAction SilentlyContinue
            if (Test-IsBotProcess $process -AllowRuntimeLockFallback) { return $process }
        }
    }
    foreach ($process in Get-CimInstance Win32_Process) {
        if (Test-IsBotProcess $process) { return $process }
    }
    return $null
}
