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
        $fullActual = [IO.Path]::GetFullPath($actualPath)
        $targetPath = [IO.Path]::GetFullPath((Join-Path $ScriptDir 'target\release\cdr-runtime.exe'))
        $bundlePath = [IO.Path]::GetFullPath((Join-Path $ScriptDir 'bin\win-x64\cdr-runtime.exe'))
        if (-not $fullActual.Equals($targetPath, [StringComparison]::OrdinalIgnoreCase) -and
            -not $fullActual.Equals($bundlePath, [StringComparison]::OrdinalIgnoreCase)) { return $null }
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

if (-not (Get-Command Set-CdrExitAllIntent -ErrorAction SilentlyContinue)) {
    foreach ($candidateRoot in @($ScriptDir, $PSScriptRoot)) {
        if ([string]::IsNullOrWhiteSpace($candidateRoot)) { continue }
        $cdrTrayJournalScript = Join-Path $candidateRoot 'scripts\CdrLaunchJournal.ps1'
        if ([IO.File]::Exists($cdrTrayJournalScript)) {
            . $cdrTrayJournalScript
            break
        }
    }
}

function Get-McpTrayStatus {
    param([string]$Root = $ScriptDir)
    try {
        $resolvedRoot = [IO.Path]::GetFullPath($Root)
        if (-not (Get-Command Resolve-CdrRemoteMcpPaths -ErrorAction SilentlyContinue)) {
            throw 'mcp_paths_resolver_unavailable'
        }
        $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $resolvedRoot
        $statusPath = $mcpPaths.StatusPath
        $legacyStatusPath = Join-Path $resolvedRoot 'bot_data\remote_mcp.status.json'
        if (-not [IO.File]::Exists($statusPath) -and -not [IO.Directory]::Exists($statusPath) -and
            ([IO.File]::Exists($legacyStatusPath) -or [IO.Directory]::Exists($legacyStatusPath))) {
            $statusPath = $legacyStatusPath
        }
        if (-not [IO.File]::Exists($statusPath) -and -not [IO.Directory]::Exists($statusPath)) {
            if (-not $mcpPaths.Enabled) {
                return [pscustomobject]@{State='not_configured';Running=$false;Ready=$false;Pid=$null;Text='Codex Remote MCP not configured or disabled';Diagnostic='mcp_not_configured_or_disabled'}
            }
            if ($mcpPaths.EffectiveOwnership -ne 'external' -or -not $mcpPaths.OwnershipCommitted) {
                return [pscustomobject]@{State='ownership_mismatch';Running=$false;Ready=$false;Pid=$null;Text='Codex Remote MCP ownership not committed external';Diagnostic='mcp_ownership_not_committed_external'}
            }
            if ([string]::IsNullOrWhiteSpace($mcpPaths.DeviceId) -or [string]::IsNullOrWhiteSpace($mcpPaths.BridgeFingerprint)) {
                throw 'mcp_config_unavailable'
            }
            if ([IO.File]::Exists($mcpPaths.InstanceLockPath) -or [IO.Directory]::Exists($mcpPaths.InstanceLockPath)) {
                $lockRec = Read-CdrMcpInstanceLockRecord -LockPath $mcpPaths.InstanceLockPath
                if ($null -ne $lockRec) {
                    $lockObs = Get-CdrProcessObservation -ProcessId $lockRec.ProcessId -ExpectedIdentity $lockRec.ProcessIdentity -ExpectedBinaryPath $lockRec.ExecutablePath
                    if ($lockObs.State -eq 'unknown') { throw 'mcp_instance_lock_process_unknown' }
                    if ($lockObs.State -eq 'running') {
                        return [pscustomobject]@{State='connecting';Running=$true;Ready=$false;Pid=$lockRec.ProcessId;Text="Codex Remote MCP connecting (PID $($lockRec.ProcessId))"}
                    }
                }
            }
            return [pscustomobject]@{State='stopped';Running=$false;Ready=$false;Pid=$null;Text='Codex Remote MCP stopped'}
        }
        $statusItem = Get-Item -LiteralPath $statusPath -ErrorAction Stop
        if ($statusItem.PSIsContainer) { throw 'mcp_status_is_directory' }
        $record = [IO.File]::ReadAllText($statusPath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
        if ($null -eq $record) { throw 'mcp_status_empty_json' }

        foreach ($reqProp in @(
            'schema_version', 'component', 'install_root', 'pid', 'process_identity',
            'executable_path', 'artifact_sha256', 'build_info', 'updated_at_ms',
            'heartbeat_at_ms', 'device_id', 'bridge_fingerprint', 'hello_ack',
            'ready', 'draining', 'generation'
        )) {
            if (-not $record.PSObject.Properties[$reqProp] -or $null -eq $record.$reqProp) {
                throw "mcp_status_missing_field:$reqProp"
            }
        }

        if (($record.schema_version -isnot [int] -and $record.schema_version -isnot [long]) -or [int]$record.schema_version -ne 1) {
            throw 'mcp_status_schema_mismatch'
        }
        if ($record.component -isnot [string] -or [string]$record.component -cne 'mcp') {
            throw 'mcp_status_component_mismatch'
        }
        if ($record.install_root -isnot [string] -or [string]::IsNullOrWhiteSpace($record.install_root)) {
            throw 'mcp_status_install_root_invalid'
        }
        $recRoot = ([string]$record.install_root).Trim().TrimEnd('/', '\').Replace('\', '/').ToLowerInvariant()
        if ($recRoot -cne $mcpPaths.NormalizedRoot) {
            throw 'mcp_status_install_root_mismatch'
        }
        if ($record.pid -isnot [int] -and $record.pid -isnot [long]) {
            throw 'mcp_status_pid_type_invalid'
        }
        $pidVal = [int]$record.pid
        if ($record.process_identity -isnot [string]) {
            throw 'mcp_status_identity_type_invalid'
        }
        $idVal = [string]$record.process_identity
        if ($pidVal -le 0 -or $idVal -notmatch '^(\d+)\|\d+$' -or [int]$Matches[1] -ne $pidVal) {
            throw 'mcp_status_identity_invalid'
        }
        if ($record.executable_path -isnot [string] -or [string]::IsNullOrWhiteSpace($record.executable_path)) {
            throw 'mcp_status_executable_path_invalid'
        }
        $exeVal = [string]$record.executable_path
        if ($record.artifact_sha256 -isnot [string] -or $record.artifact_sha256 -notmatch '^[0-9a-fA-F]{64}$') {
            throw 'mcp_status_artifact_sha256_invalid'
        }
        if ($record.build_info -isnot [pscustomobject] -or
            -not $record.build_info.PSObject.Properties['schema_version'] -or
            ($record.build_info.schema_version -isnot [int] -and $record.build_info.schema_version -isnot [long]) -or
            [int]$record.build_info.schema_version -ne 1 -or
            -not $record.build_info.PSObject.Properties['package_version'] -or
            $record.build_info.package_version -isnot [string] -or
            [string]::IsNullOrWhiteSpace($record.build_info.package_version) -or
            -not $record.build_info.PSObject.Properties['source_state'] -or
            $record.build_info.source_state -isnot [string] -or
            [string]::IsNullOrWhiteSpace($record.build_info.source_state)) {
            throw 'mcp_status_build_info_invalid'
        }
        if (($record.updated_at_ms -isnot [int] -and $record.updated_at_ms -isnot [long]) -or
            ($record.heartbeat_at_ms -isnot [int] -and $record.heartbeat_at_ms -isnot [long])) {
            throw 'mcp_status_timestamp_type_invalid'
        }
        if ($record.device_id -isnot [string] -or [string]::IsNullOrWhiteSpace($record.device_id)) {
            throw 'mcp_status_device_id_invalid'
        }
        if ($record.bridge_fingerprint -isnot [string] -or [string]::IsNullOrWhiteSpace($record.bridge_fingerprint)) {
            throw 'mcp_status_bridge_fingerprint_invalid'
        }
        if (($record.generation -isnot [int] -and $record.generation -isnot [long]) -or [int64]$record.generation -le 0) {
            throw 'mcp_status_generation_invalid'
        }
        if ($record.ready -isnot [bool] -or $record.hello_ack -isnot [bool] -or $record.draining -isnot [bool]) {
            throw 'mcp_status_boolean_fields_invalid'
        }

        if (-not $mcpPaths.Enabled) {
            return [pscustomobject]@{State='not_configured';Running=$false;Ready=$false;Pid=$null;Text='Codex Remote MCP not configured or disabled';Diagnostic='mcp_not_configured_or_disabled'}
        }
        if ($mcpPaths.EffectiveOwnership -ne 'external' -or -not $mcpPaths.OwnershipCommitted) {
            return [pscustomobject]@{State='ownership_mismatch';Running=$false;Ready=$false;Pid=$null;Text='Codex Remote MCP ownership not committed external';Diagnostic='mcp_ownership_not_committed_external'}
        }
        if ([string]::IsNullOrWhiteSpace($mcpPaths.DeviceId) -or [string]::IsNullOrWhiteSpace($mcpPaths.BridgeFingerprint)) {
            throw 'mcp_config_unavailable'
        }

        $obs = Get-CdrProcessObservation -ProcessId $pidVal -ExpectedIdentity $idVal -ExpectedBinaryPath $exeVal
        if ($obs.State -eq 'unknown') {
            throw "mcp_process_observation_unknown:$($obs.Reason)"
        }
        if ($obs.State -eq 'stopped') {
            return [pscustomobject]@{State='stopped';Running=$false;Ready=$false;Pid=$null;Text='Codex Remote MCP stopped'}
        }
        if ([IO.File]::Exists($mcpPaths.InstanceLockPath) -or [IO.Directory]::Exists($mcpPaths.InstanceLockPath)) {
            $lockRec = Read-CdrMcpInstanceLockRecord -LockPath $mcpPaths.InstanceLockPath
            if ($null -ne $lockRec -and $lockRec.ProcessIdentity -cne $idVal) {
                return [pscustomobject]@{State='mismatch';Running=$true;Ready=$false;Pid=$pidVal;Text='Codex Remote MCP lock owner mismatch';Diagnostic='mcp_lock_owner_mismatch'}
            }
        }
        if ([string]$record.device_id -cne $mcpPaths.DeviceId) {
            return [pscustomobject]@{State='mismatch';Running=$true;Ready=$false;Pid=$pidVal;Text='Codex Remote MCP device mismatch';Diagnostic='mcp_device_mismatch'}
        }
        if ([string]$record.bridge_fingerprint -cne $mcpPaths.BridgeFingerprint) {
            return [pscustomobject]@{State='mismatch';Running=$true;Ready=$false;Pid=$pidVal;Text='Codex Remote MCP bridge mismatch';Diagnostic='mcp_bridge_mismatch'}
        }
        if (-not [IO.File]::Exists($exeVal)) {
            return [pscustomobject]@{State='mismatch';Running=$true;Ready=$false;Pid=$pidVal;Text='Codex Remote MCP executable missing'}
        }
        $actualHash = (Get-FileHash -LiteralPath $exeVal -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualHash -cne ([string]$record.artifact_sha256).ToLowerInvariant()) {
            return [pscustomobject]@{State='mismatch';Running=$true;Ready=$false;Pid=$pidVal;Text='Codex Remote MCP artifact mismatch'}
        }
        $nowMs = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $ageMs = $nowMs - [int64]$record.updated_at_ms
        $hbAgeMs = $nowMs - [int64]$record.heartbeat_at_ms
        $connStale = $false
        $hasFreshConnEvidence = $false
        if ($record.PSObject.Properties['connection_verified_at_ms'] -and $null -ne $record.connection_verified_at_ms) {
            if ($record.connection_verified_at_ms -isnot [int] -and $record.connection_verified_at_ms -isnot [long]) {
                return [pscustomobject]@{State='unknown';Running=$false;Ready=$false;Pid=$null;Text='Codex Remote MCP status unavailable';Diagnostic='invalid_status_schema'}
            }
            $connAgeMs = $nowMs - [int64]$record.connection_verified_at_ms
            if ([int64]$record.connection_verified_at_ms -le 0 -or $connAgeMs -lt -2000 -or
                ([int64]$record.connection_verified_at_ms -gt ([int64]$record.heartbeat_at_ms + 2000)) -or
                $connAgeMs -gt 6000) {
                $connStale = $true
            } else {
                $hasFreshConnEvidence = $true
            }
        } elseif ($record.ready -or $record.hello_ack) {
            $connStale = $true
        }
        if ([int64]$record.updated_at_ms -le 0 -or [int64]$record.heartbeat_at_ms -le 0 -or
            $ageMs -lt -2000 -or $hbAgeMs -lt -2000 -or
            ([int64]$record.updated_at_ms -gt ([int64]$record.heartbeat_at_ms + 2000)) -or
            $ageMs -gt 6000 -or $hbAgeMs -gt 6000 -or $connStale) {
            return [pscustomobject]@{State='stale';Running=$true;Ready=$false;Pid=$pidVal;Text="Codex Remote MCP stale (PID $pidVal)"}
        }
        $ready = [bool]($record.ready -and $record.hello_ack -and (-not $record.draining) -and ([int64]$record.generation -ge 1) -and $hasFreshConnEvidence)
        if ($ready) {
            return [pscustomobject]@{State='running';Running=$true;Ready=$true;Pid=$pidVal;Text="Codex Remote MCP ready (PID $pidVal)"}
        }
        return [pscustomobject]@{State='connecting';Running=$true;Ready=$false;Pid=$pidVal;Text="Codex Remote MCP connecting (PID $pidVal)"}
    } catch {
        return [pscustomobject]@{State='unknown';Running=$false;Ready=$false;Pid=$null;Text='Codex Remote MCP status unavailable';Diagnostic=$_.Exception.Message}
    }
}

function Start-CdrTrayStandAlone {
    param([string]$Root = $ScriptDir)
    try {
        $resolved = [IO.Path]::GetFullPath($Root).TrimEnd('\')
        if (-not (Test-CdrTrayInteractiveSession)) {
            return [pscustomobject]@{State='noninteractive';Pid=$null}
        }
        if (Get-CdrTrayMutexBusy (Get-CdrTrayMutexName $resolved)) {
            return [pscustomobject]@{State='owner_present';Pid=$null}
        }
        $trayPath = Join-Path $resolved 'codex-discord-tray.ps1'
        $shellPath = Join-Path ([Environment]::GetFolderPath('System')) 'WindowsPowerShell\v1.0\powershell.exe'
        if (-not (Test-Path -LiteralPath $trayPath -PathType Leaf)) { throw 'tray_script_missing' }
        $child = Start-CdrDetachedProcess -FilePath $shellPath -WorkingDirectory $resolved -WindowStyle 'Hidden' `
            -ArgumentList @('-NoProfile','-STA','-ExecutionPolicy','Bypass','-WindowStyle','Hidden','-File',$trayPath)
        try {
            if ($null -eq $child -or [int]$child.Id -le 0) {
                return [pscustomobject]@{State='unknown';Pid=$null}
            }
            return [pscustomobject]@{State='spawn_requested';Pid=[int]$child.Id}
        } finally {
            if ($child -is [IDisposable]) { $child.Dispose() }
        }
    } catch {
        return [pscustomobject]@{State='unknown';Pid=$null;Diagnostic=$_.Exception.GetType().FullName}
    }
}

function Request-BotStart {
    param([string]$Root = $ScriptDir)
    $resolved = [IO.Path]::GetFullPath($Root)
    if (Get-Command Set-CdrComponentIntent -ErrorAction SilentlyContinue) {
        Set-CdrComponentIntent -RepoRoot $resolved -Component 'bot' -DesiredState 'running' -Action 'tray-start-bot' -Source 'tray' | Out-Null
    } else {
        foreach ($name in @('.codex_discord_rust.stop', '.codex_discord_bot.disabled')) {
            $p = Join-Path $resolved $name
            if ([IO.File]::Exists($p)) { [IO.File]::Delete($p) }
        }
    }
    $watchdogPath = Join-Path $resolved 'codex-discord-rust-watchdog.ps1'
    if (-not [IO.File]::Exists($watchdogPath)) { throw "Rust watchdog script not found: $watchdogPath" }
    $shellPath = Join-Path ([Environment]::GetFolderPath('System')) 'WindowsPowerShell\v1.0\powershell.exe'
    $args = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$watchdogPath,'-RepoRoot',$resolved)
    Start-CdrDetachedProcess -FilePath $shellPath -ArgumentList $args -WorkingDirectory $resolved -WindowStyle 'Hidden' | Out-Null
}

function Request-BotStop {
    param([string]$Root = $ScriptDir, [string]$BotBinaryPath = '', [int]$GraceTimeoutMs = 45000)
    $resolved = [IO.Path]::GetFullPath($Root)
    return (Stop-CdrComponentRuntime -RepoRoot $resolved -Component 'bot' -BinaryPath $BotBinaryPath -GraceTimeoutMs $GraceTimeoutMs -Source 'tray-stop-bot')
}

function Invoke-CdrTrayExitAll {
    param(
        [string]$Root = $ScriptDir,
        [string]$BotBinaryPath = '',
        [string]$McpBinaryPath = '',
        [int]$GraceTimeoutMs = 45000,
        [scriptblock]$BotKillOverride = $null,
        [scriptblock]$McpKillOverride = $null,
        [scriptblock]$OnAfterBotStop = $null
    )
    $resolved = [IO.Path]::GetFullPath($Root)
    # 1. Persist stop intent for BOTH bot and MCP first under the common intent lock so no watchdog or delayed helper can restart either component.
    $exitDecision = Set-CdrExitAllIntent -RepoRoot $resolved -Source 'tray-exit-all'
    $expectedBotRev = [uint64]$exitDecision.Bot.Revision
    $expectedMcpRev = [uint64]$exitDecision.Mcp.Revision

    # 2. Stop both components gracefully and independently using verified process identity and exact per-component Exit-All revisions.
    $botRes = Stop-CdrComponentRuntime -RepoRoot $resolved -Component 'bot' -BinaryPath $BotBinaryPath `
        -GraceTimeoutMs $GraceTimeoutMs -Source 'tray-exit-all' -PreserveIntent `
        -ExpectedIntentRevision $expectedBotRev -ExpectedDesiredState 'stopped' -KillOverride $BotKillOverride
    if ($null -ne $OnAfterBotStop) {
        & $OnAfterBotStop $exitDecision $botRes
    }
    $mcpRes = Stop-CdrComponentRuntime -RepoRoot $resolved -Component 'mcp' -BinaryPath $McpBinaryPath `
        -GraceTimeoutMs $GraceTimeoutMs -Source 'tray-exit-all' -PreserveIntent `
        -ExpectedIntentRevision $expectedMcpRev -ExpectedDesiredState 'stopped' -KillOverride $McpKillOverride

    # 3. Re-verify under the intent lock that neither component's Exit-All intent was superseded and both components are actually stopped.
    $botStoppedFinal = [bool]($botRes.Stopped -and $botRes.State -eq 'stopped')
    $mcpStoppedFinal = [bool]($mcpRes.Stopped -and $mcpRes.State -eq 'stopped')
    $botStateFinal = [string]$botRes.State
    $mcpStateFinal = [string]$mcpRes.State
    $verifyLock = Enter-CdrIntentLock -RepoRoot $resolved
    try {
        $finalBotIntent = Get-CdrComponentIntent -RepoRoot $resolved -Component 'bot'
        $finalMcpIntent = Get-CdrComponentIntent -RepoRoot $resolved -Component 'mcp'
        $finalBotObs = Get-CdrComponentObservation -RepoRoot $resolved -Component 'bot' -BinaryPath $BotBinaryPath
        $finalMcpObs = Get-CdrComponentObservation -RepoRoot $resolved -Component 'mcp' -BinaryPath $McpBinaryPath

        $botIntentMatches = ([uint64]$finalBotIntent.Revision -eq $expectedBotRev -and [string]$finalBotIntent.DesiredState -ceq 'stopped')
        $mcpIntentMatches = ([uint64]$finalMcpIntent.Revision -eq $expectedMcpRev -and [string]$finalMcpIntent.DesiredState -ceq 'stopped')

        if (-not $botIntentMatches -or $finalBotObs.State -ne 'stopped') {
            $botStoppedFinal = $false
            $botStateFinal = if (-not $botIntentMatches -and $finalBotObs.State -eq 'running') {
                'still_running'
            } elseif (-not $botIntentMatches) {
                'superseded_by_newer_intent'
            } elseif ($finalBotObs.State -eq 'running') {
                'still_running'
            } else {
                [string]$finalBotObs.State
            }
        }
        if (-not $mcpIntentMatches -or $finalMcpObs.State -ne 'stopped') {
            $mcpStoppedFinal = $false
            $mcpStateFinal = if (-not $mcpIntentMatches -and $finalMcpObs.State -eq 'running') {
                'still_running'
            } elseif (-not $mcpIntentMatches) {
                'superseded_by_newer_intent'
            } elseif ($finalMcpObs.State -eq 'running') {
                'still_running'
            } else {
                [string]$finalMcpObs.State
            }
        }
    } finally {
        Exit-CdrIntentLock $verifyLock
    }

    $allStopped = [bool]($botStoppedFinal -and $mcpStoppedFinal -and $botStateFinal -eq 'stopped' -and $mcpStateFinal -eq 'stopped')
    return [pscustomobject]@{
        AllStopped   = $allStopped
        BotStopped   = $botStoppedFinal
        McpStopped   = $mcpStoppedFinal
        BotState     = $botStateFinal
        McpState     = $mcpStateFinal
        KeepTrayOpen = (-not $allStopped)
        Diagnostic   = if ($allStopped) { 'exit_all_clean' } else { "exit_all_partial_failure:bot=$botStateFinal,mcp=$mcpStateFinal" }
    }
}
