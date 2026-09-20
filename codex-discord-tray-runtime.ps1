function Resolve-TrayRuntimeMode {
    $mode = [string]$env:CODEX_DISCORD_RUNTIME
    $modePath = Join-Path $ScriptDir '.codex_discord_runtime'
    if ([string]::IsNullOrWhiteSpace($mode) -and (Test-Path -LiteralPath $modePath)) {
        $mode = (Get-Content -LiteralPath $modePath -Raw -Encoding UTF8).Trim()
    }
    if ([string]::IsNullOrWhiteSpace($mode)) { return 'rust' }
    if ($mode -eq 'rust') { return $mode.ToLowerInvariant() }
    throw "Unsupported Codex Discord runtime selection: '$mode'."
}

function Get-RustTrayProcess {
    $lockPath = Join-Path $ScriptDir '.codex_discord_rust.runtime.lock'
    # Absence is stopped; an observation failure is unknown, not restart authority.
    try { $item = Get-Item -LiteralPath $lockPath -ErrorAction Stop }
    catch [Management.Automation.ItemNotFoundException] { return $null }
    if ($null -eq $item -or $item.PSIsContainer) { throw 'tray_lock_not_observable_file' }
    $lockText = Get-Content -LiteralPath $lockPath -Raw -Encoding UTF8 -ErrorAction Stop
    # Count ALL pid fields before validating the sole value (including bad duplicates).
    $pidFields = [regex]::Matches([string]$lockText, '(?m)^pid=[^\r\n]*\r?$')
    $processId = 0
    if ($pidFields.Count -ne 1 -or $pidFields[0].Value -notmatch '\Apid=([1-9][0-9]*)\r?\z' -or
        -not [int]::TryParse($Matches[1], [ref]$processId)) { throw 'tray_lock_pid_unavailable' }
    try { $process = Get-Process -Id $processId -ErrorAction Stop }
    catch {
        if ($_.FullyQualifiedErrorId -ceq 'NoProcessFoundForGivenId,Microsoft.PowerShell.Commands.GetProcessCommand' -and
            $_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) { return $null }
        throw
    }
    if ($null -eq $process) { throw 'tray_process_observation_empty' }
    try {
        if ([int]$process.Id -ne $processId) { throw 'tray_process_id_mismatch' }
        $actualPath = [string]$process.Path
        if ([string]::IsNullOrWhiteSpace($actualPath)) { throw 'tray_process_path_unavailable' }
        $expectedPath = [IO.Path]::GetFullPath((Join-Path $ScriptDir 'target\release\cdr-runtime.exe'))
        if (-not [IO.Path]::GetFullPath($actualPath).Equals($expectedPath, [StringComparison]::OrdinalIgnoreCase)) { return $null }
        $started = $process.StartTime
        if ($started -isnot [datetime] -or $started -eq [datetime]::MinValue) { throw 'tray_process_start_time_unavailable' }
        return [pscustomobject]@{ ProcessId=$processId; StartTime=$started }
    } finally {
        if ($process -is [IDisposable]) { $process.Dispose() }
    }
}

function Get-RustTrayProcessIdentity {
    param($Process)
    if ($null -eq $Process) { return '' }
    $ticks = $Process.StartTime.ToUniversalTime().Ticks
    $ticks -= ($ticks % 10)
    return "$([int]$Process.ProcessId)|$ticks"
}

function Get-BotProcess {
    if ($RuntimeMode -ne 'rust') { throw 'Tray runtime mode was not initialized as Rust.' }
    return Get-RustTrayProcess
}

# Status observation grants no execution/restart authority.
function Get-BotStatus {
    try {
        $process = Get-BotProcess
        if ($null -eq $process) {
            return [pscustomobject]@{State='stopped';Running=$false;Pid=$null;Text='Codex Discord bridge stopped';Icon='Warning'}
        }
        return [pscustomobject]@{State='running';Running=$true;Pid=[int]$process.ProcessId;Text="Codex Discord bridge running (PID $($process.ProcessId))";Icon='Application'}
    } catch {
        return [pscustomobject]@{State='unknown';Running=$false;Pid=$null;Text='Codex Discord bridge status unavailable';Icon='Warning';Diagnostic="read_failed type=$($_.Exception.GetType().FullName)"}
    }
}

function Get-CdrTrayMutexName {
    param([string]$Root)
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes([IO.Path]::GetFullPath($Root).TrimEnd('\').ToLowerInvariant())
        $suffix = [BitConverter]::ToString($hash.ComputeHash($bytes)).Replace('-', '').Substring(0, 16)
        return "Local\CodexDiscordTray_$suffix"
    } finally { $hash.Dispose() }
}

function Get-CdrTrayMutexBusy {
    param([string]$Name)
    $mutex = $null; $owned = $false
    try {
        try { $mutex = [Threading.Mutex]::OpenExisting($Name) }
        catch [Threading.WaitHandleCannotBeOpenedException] { return $false }
        try { $owned = $mutex.WaitOne(0) }
        catch [Threading.AbandonedMutexException] { $owned = $true }
        return (-not $owned)
    } finally {
        if ($null -ne $mutex) {
            try { if ($owned) { $mutex.ReleaseMutex() } }
            finally { $mutex.Dispose() }
        }
    }
}

function Test-CdrTrayInteractiveSession {
    $current = [Diagnostics.Process]::GetCurrentProcess()
    try { $session = $current.SessionId } finally { $current.Dispose() }
    if ($session -le 0 -or -not [Environment]::UserInteractive) { return $false }
    return (@(Get-Process -Name explorer -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session }).Count -gt 0)
}

function Start-CdrTrayForRuntime {
    param([string]$Root, [string]$ExpectedRuntimeIdentity)
    # Best-effort UI only. No bot launcher, markers, database, Discord, or retry.
    try {
        $resolved = [IO.Path]::GetFullPath($Root).TrimEnd('\')
        $ownRoot = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
        if (-not $resolved.Equals($ownRoot, [StringComparison]::OrdinalIgnoreCase)) {
            return [pscustomobject]@{State='root_mismatch';Pid=$null}
        }
        if ($ExpectedRuntimeIdentity -notmatch '^\d+\|\d+$') {
            return [pscustomobject]@{State='runtime_mismatch';Pid=$null}
        }
        if (-not (Test-CdrTrayInteractiveSession)) {
            return [pscustomobject]@{State='noninteractive';Pid=$null}
        }
        $ScriptDir = $resolved
        $RuntimeMode = Resolve-TrayRuntimeMode
        $process = Get-RustTrayProcess
        if ((Get-RustTrayProcessIdentity $process) -cne $ExpectedRuntimeIdentity) {
            return [pscustomobject]@{State='runtime_mismatch';Pid=$null}
        }
        if (Get-CdrTrayMutexBusy (Get-CdrTrayMutexName $resolved)) {
            return [pscustomobject]@{State='owner_present';Pid=$null}
        }
        $trayPath = Join-Path $resolved 'codex-discord-tray.ps1'
        $shellPath = Join-Path ([Environment]::GetFolderPath('System')) 'WindowsPowerShell\v1.0\powershell.exe'
        if (-not (Test-Path -LiteralPath $trayPath -PathType Leaf)) { throw 'tray_script_missing' }
        # Recheck after session/mutex/file observations. Child UI enforces singleton.
        if ((Get-RustTrayProcessIdentity (Get-RustTrayProcess)) -cne $ExpectedRuntimeIdentity) {
            return [pscustomobject]@{State='runtime_mismatch';Pid=$null}
        }
        if (Test-CdrTrayControlPending $resolved) { return [pscustomobject]@{State='control_pending';Pid=$null} }
        $child = Start-Process -FilePath $shellPath -WorkingDirectory $resolved -WindowStyle Hidden -PassThru `
            -ArgumentList @('-NoProfile','-STA','-ExecutionPolicy','Bypass','-WindowStyle','Hidden','-File',('"'+$trayPath+'"'))
        # A PID proves spawn only, not a visible icon or a healthy UI message loop.
        try {
            if ($null -eq $child -or [int]$child.Id -le 0) {
                return [pscustomobject]@{State='unknown';Pid=$null}
            }
            return [pscustomobject]@{State='spawn_requested';Pid=[int]$child.Id}
        } finally {
            if ($child -is [IDisposable]) { $child.Dispose() }
        }
    } catch {
        # Start-Process might have created a child before throwing. Never retry here.
        return [pscustomobject]@{State='unknown';Pid=$null;Diagnostic=$_.Exception.GetType().FullName}
    }
}


# UI-only delivery. These records never authorize bot control or work replay.
function Test-CdrTrayControlPending {
    param([string]$Root)
    foreach ($name in @('.codex_discord_bot.disabled','.codex_discord_rust.stop',
        '.codex_discord_rust.restart','.codex_discord_rust.restart.launch',
        '.codex_discord_rust.force.launch',
        '.codex_discord_rust.drain.prepare','.codex_discord_rust.drain.ack',
        '.codex_discord_rust.maintenance.v2')) {
        try { $null=Get-Item -LiteralPath (Join-Path $Root $name) -ErrorAction Stop }
        catch [Management.Automation.ItemNotFoundException] { continue }
        return $true
    }
    return ([IO.Directory]::GetFiles($Root,'.codex_discord_rust.restart.claimed.*').Count -ne 0)
}

function Get-CdrTrayCompletedIntent {
    param([string]$Root,[string]$ExpectedIdentity)
    foreach ($kind in @('maintenance.v2','restart')) {
        $path=Join-Path $Root ('.codex_discord_rust.'+$kind+'.completed')
        try { $item=Get-Item -LiteralPath $path -ErrorAction Stop }
        catch [Management.Automation.ItemNotFoundException] { continue }
        if ($null -eq $item -or $item.PSIsContainer -or $item.Length -gt 65536) { throw 'tray_completion_unreadable' }
        $record=Get-Content -LiteralPath $path -Raw -Encoding UTF8 -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
        # Legacy receipts have no explicit UI intent. Do not resurrect a closed tray.
        if ([string]$record.TrayBootstrapIdentity -cne $ExpectedIdentity) { continue }
        if ($kind -eq 'maintenance.v2') {
            if ($record.Version -ne 2 -or $record.Phase -cne 'verified' -or
                [string]$record.RepoRoot -ine $Root -or
                [string]$record.BinaryPath -ine (Join-Path $Root 'target\release\cdr-runtime.exe')) { continue }
            if ($record.CompletionPolicy -ceq 'runtime-proof-v1' -and
                [string]$record.RuntimeEvidence.ChildIdentity -cne $ExpectedIdentity) { continue }
        } elseif ([string]$record.TrayBootstrapRoot -ine $Root -or
                  [string]$record.ReplacementIdentity -cne $ExpectedIdentity) { continue }
        return $ExpectedIdentity
    }
    return ''
}

function Invoke-CdrTrayAfterWatchdog {
    param([string]$Root,[string]$StartedIdentity,[string]$ObservedIdentity)
    try {
        $resolved=[IO.Path]::GetFullPath($Root).TrimEnd('\')
        if (-not $resolved.Equals([IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\'),[StringComparison]::OrdinalIgnoreCase)) {
            return [pscustomobject]@{State='root_mismatch';Pid=$null}
        }
        $expected=if($StartedIdentity){$StartedIdentity}else{$ObservedIdentity}
        if ($expected -notmatch '^[1-9][0-9]*\|[1-9][0-9]*$') { return [pscustomobject]@{State='no_request';Pid=$null} }
        if (Test-CdrTrayControlPending $resolved) { return [pscustomobject]@{State='control_pending';Pid=$null} }
        if (-not $StartedIdentity -and (Get-CdrTrayCompletedIntent $resolved $expected) -cne $expected) {
            return [pscustomobject]@{State='no_request';Pid=$null}
        }
        $ScriptDir=$resolved
        if ((Get-RustTrayProcessIdentity (Get-RustTrayProcess)) -cne $expected) {
            return [pscustomobject]@{State='runtime_mismatch';Pid=$null}
        }
        # Atomic cross-process one-attempt fence. Empty/partial claims also consume
        # the UI attempt. Never delete or overwrite this after unknown outcomes.
        $claimPath=Join-Path $resolved ('.codex_discord_tray.bootstrap.'+$expected.Replace('|','.')+'.attempted')
        try { $claim=[IO.File]::Open($claimPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None) }
        catch [IO.IOException] {
            if ([IO.File]::Exists($claimPath)) { return [pscustomobject]@{State='already_attempted';Pid=$null} }
            throw
        }
        try {
            $bytes=[Text.Encoding]::UTF8.GetBytes("version=1`nidentity=$expected`n")
            $claim.Write($bytes,0,$bytes.Length)
            $claim.Flush($true)
        } finally { $claim.Dispose() }
        if (Test-CdrTrayControlPending $resolved) { return [pscustomobject]@{State='control_pending';Pid=$null} }
        return Start-CdrTrayForRuntime -Root $resolved -ExpectedRuntimeIdentity $expected
    } catch { return [pscustomobject]@{State='unknown';Pid=$null;Diagnostic=$_.Exception.GetType().FullName} }
}
