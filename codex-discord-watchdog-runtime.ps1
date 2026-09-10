$GracefulStopTimeoutSeconds = 60
$RestartWaitPollSeconds = 5
$Sha256 = [Security.Cryptography.SHA256]::Create()
try {
    $ScriptDirHash = $Sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($ScriptDir.ToLowerInvariant()))
    $ScriptDirHashText = (($ScriptDirHash | ForEach-Object { $_.ToString('x2') }) -join '')
    $RuntimeMutexName = 'Local\CodexDiscordBot_' + $ScriptDirHashText.Substring(0, 16)
} finally {
    $Sha256.Dispose()
}

function Write-LauncherLog {
    param([string]$Message)

    $timestamp = (Get-Date).ToString('s')
    Add-Content -LiteralPath $LauncherLogPath -Encoding UTF8 -Value "[$timestamp] $Message"
}

function Get-RuntimePid {
    if (-not (Test-Path -LiteralPath $RuntimeLockPath)) {
        return $null
    }

    try {
        $runtimePidText = (Get-Content -LiteralPath $RuntimeLockPath -Raw -ErrorAction Stop).Trim()
        if ($runtimePidText -notmatch '^\d+$') {
            Write-LauncherLog "runtime_lock_invalid lock=$RuntimeLockPath value=$runtimePidText"
            return $null
        }
        return [int]$runtimePidText
    } catch {
        Write-LauncherLog "runtime_lock_probe_failed lock=$RuntimeLockPath error=$($_.Exception.GetType().Name)"
        return $null
    }
}

function Test-IsBotProcess {
    param(
        $Process,
        [switch]$AllowRuntimeLockFallback
    )

    if ($Process -eq $null) {
        return $false
    }
    $name = [string]$Process.Name
    if ($name -ne 'py.exe' -and $name -ne 'python.exe' -and $name -ne 'pythonw.exe') {
        return $false
    }
    $needle = [IO.Path]::GetFullPath($BotScript).ToLowerInvariant()
    $commandLine = ([string]$Process.CommandLine).ToLowerInvariant()
    if (-not $commandLine -and $AllowRuntimeLockFallback) {
        return $true
    }
    return $commandLine.Contains($needle)
}

function Get-BotProcesses {
    foreach ($process in Get-CimInstance Win32_Process) {
        if (Test-IsBotProcess $process) {
            $process
        }
    }
}

function Get-BotProcessStartTime {
    $runtimePid = Get-RuntimePid
    if ($runtimePid -ne $null) {
        $runtimeProcess = Get-CimInstance Win32_Process -Filter "ProcessId=$runtimePid" -ErrorAction SilentlyContinue
        if (Test-IsBotProcess $runtimeProcess -AllowRuntimeLockFallback) {
            return [datetime]$runtimeProcess.CreationDate
        }
    }

    $process = Get-BotProcesses |
        Sort-Object -Property CreationDate |
        Select-Object -First 1
    if ($process -eq $null) {
        return $null
    }
    return [datetime]$process.CreationDate
}

function Get-BotProcessAgeSeconds {
    $runtimePid = Get-RuntimePid
    if ($runtimePid -eq $null) {
        $process = Get-BotProcesses |
            Sort-Object -Property CreationDate |
            Select-Object -First 1
        if ($process -eq $null) {
            return $null
        }
        $runtimePid = [int]$process.ProcessId
    }
    try {
        $performance = Get-CimInstance `
            Win32_PerfFormattedData_PerfProc_Process `
            -Filter "IDProcess=$runtimePid" `
            -ErrorAction Stop |
            Select-Object -First 1
        if ($performance -eq $null -or $performance.ElapsedTime -eq $null) {
            return $null
        }
        return [math]::Max(0, [math]::Floor([double]$performance.ElapsedTime))
    } catch {
        Write-LauncherLog "runtime_age_probe_failed pid=$runtimePid error=$($_.Exception.GetType().Name)"
        return $null
    }
}

function Test-RuntimeMutexHeld {
    $created = $false
    $mutex = $null
    try {
        $mutex = [Threading.Mutex]::new($true, $RuntimeMutexName, [ref]$created)
        return -not $created
    } catch {
        Write-LauncherLog "runtime_mutex_probe_failed mutex=$RuntimeMutexName error=$($_.Exception.GetType().Name)"
        return $false
    } finally {
        if ($mutex -ne $null) {
            if ($created) {
                $mutex.ReleaseMutex()
            }
            $mutex.Dispose()
        }
    }
}

function Wait-RuntimeBotExit {
    param(
        [string]$ExpectedIdentity,
        [int]$TimeoutSeconds = $GracefulStopTimeoutSeconds
    )

    for ($i = 0; $i -lt $TimeoutSeconds; $i++) {
        Start-Sleep -Seconds 1
        $expectedPid = [int]($ExpectedIdentity -split '\|', 2)[0]
        $currentIdentity = Get-CodexBotProcessIdentityById `
            -BotScript $BotScript `
            -ProcessId $expectedPid
        if ($currentIdentity -ne $ExpectedIdentity) {
            return $true
        }
    }
    return $false
}

function Request-GracefulRuntimeStop {
    param(
        [string]$ExpectedIdentity,
        [switch]$PreserveRemoteMcpContext
    )

    $expectedPid = [int]($ExpectedIdentity -split '\|', 2)[0]
    $currentIdentity = Get-CodexBotProcessIdentityById `
        -BotScript $BotScript `
        -ProcessId $expectedPid
    if ($currentIdentity -ne $ExpectedIdentity) {
        Write-LauncherLog "watchdog_graceful_stop_stale expected=$ExpectedIdentity current=$currentIdentity"
        return $true
    }
    $stopContent = "identity=$ExpectedIdentity"
    if ($PreserveRemoteMcpContext) {
        $stopContent += "`nmode=restart"
    }
    try {
        Publish-AtomicTextFile `
            -Path $StopRequestPath `
            -Content $stopContent
    } catch {
        Write-LauncherLog "watchdog_graceful_stop_marker_failed marker=$StopRequestPath error=$($_.Exception.GetType().Name)"
        if ($PreserveRemoteMcpContext) {
            throw
        }
        return $false
    }
    Write-LauncherLog "watchdog_graceful_stop_requested marker=$StopRequestPath"
    if (Wait-RuntimeBotExit `
        -ExpectedIdentity $ExpectedIdentity `
        -TimeoutSeconds $GracefulStopTimeoutSeconds) {
        Remove-Item -LiteralPath $StopRequestPath -Force -ErrorAction SilentlyContinue
        Write-LauncherLog "watchdog_graceful_stop_done marker=$StopRequestPath"
        return $true
    }
    Write-LauncherLog "watchdog_graceful_stop_timeout marker=$StopRequestPath"
    if ($PreserveRemoteMcpContext) {
        Remove-Item -LiteralPath $StopRequestPath -Force -ErrorAction SilentlyContinue
        throw 'Remote MCP restart handoff did not finish; refusing forced restart.'
    }
    return $false
}

function Stop-RuntimeBotProcess {
    param(
        [string]$ExpectedIdentity,
        [switch]$PreserveRemoteMcpContext
    )

    if ($ExpectedIdentity -notmatch '^\d+\|\d+$') {
        throw 'A verified bot process identity is required before stopping the runtime.'
    }
    if (Request-GracefulRuntimeStop `
        -ExpectedIdentity $ExpectedIdentity `
        -PreserveRemoteMcpContext:$PreserveRemoteMcpContext) {
        return
    }
    Remove-Item -LiteralPath $StopRequestPath -Force -ErrorAction SilentlyContinue

    $expectedPid = [int]($ExpectedIdentity -split '\|', 2)[0]
    $ownedProcess = Get-Process -Id $expectedPid -ErrorAction SilentlyContinue
    if ($ownedProcess -eq $null) {
        Write-LauncherLog "watchdog_restart_expected_process_gone identity=$ExpectedIdentity"
        return
    }
    try {
        $retainedHandle = $ownedProcess.Handle
        $startedAt = $ownedProcess.StartTime.ToUniversalTime()
        $startedTicks = $startedAt.Ticks - ($startedAt.Ticks % 10)
        $ownedIdentity = "$expectedPid|$startedTicks"
        $verifiedIdentity = Get-CodexBotProcessIdentityById `
            -BotScript $BotScript `
            -ProcessId $expectedPid
        if (
            $ownedIdentity -ne $ExpectedIdentity -or
            $verifiedIdentity -ne $ExpectedIdentity
        ) {
            Write-LauncherLog (
                "watchdog_restart_exact_identity_mismatch " +
                "expected=$ExpectedIdentity owned=$ownedIdentity verified=$verifiedIdentity"
            )
            return
        }
        Write-LauncherLog "watchdog_restart_stop_exact identity=$ExpectedIdentity handle=$retainedHandle"
        $ownedProcess.Kill()
        if (-not $ownedProcess.WaitForExit(5000)) {
            throw "Timed out waiting for exact bot process to exit: $ExpectedIdentity"
        }
    } finally {
        $ownedProcess.Dispose()
    }
}

function Test-BotProcessAlive {
    if (-not (Test-Path -LiteralPath $BotScript)) {
        return $false
    }

    $runtimePid = Get-RuntimePid
    if ($runtimePid -ne $null) {
        $runtimeProcess = Get-CimInstance Win32_Process -Filter "ProcessId=$runtimePid" -ErrorAction SilentlyContinue
        if (Test-IsBotProcess $runtimeProcess -AllowRuntimeLockFallback) {
            return $true
        } elseif ($runtimeProcess -ne $null) {
            Write-LauncherLog "runtime_lock_pid_mismatch pid=$runtimePid name=$($runtimeProcess.Name)"
            Remove-Item -LiteralPath $RuntimeLockPath -Force -ErrorAction SilentlyContinue
        }
    }

    foreach ($process in Get-BotProcesses) {
        if ($process -ne $null) {
            return $true
        }
    }
    if (Test-RuntimeMutexHeld) {
        Write-LauncherLog "runtime_mutex_alive mutex=$RuntimeMutexName lock=$RuntimeLockPath"
        return $true
    }
    return $false
}
