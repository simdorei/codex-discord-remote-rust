if (-not (Get-Command Get-CdrProcessObservation -ErrorAction SilentlyContinue)) {
    $cdrIdentityModule = Join-Path $PSScriptRoot 'CdrRuntimeIdentity.psm1'
    if ([IO.File]::Exists($cdrIdentityModule)) {
        Import-Module $cdrIdentityModule -Force
    }
}

# Durable launch ownership. Completion receipts are derived from this record,
# never from whichever process happens to own the runtime lock at a later time.
function Read-CdrLaunchJournal {
    param([string]$Path)
    if (-not [IO.File]::Exists($Path)) { return $null }
    $record = [IO.File]::ReadAllText($Path) | ConvertFrom-Json
    if ($record.BinaryPath -cne $BinaryPath -or $record.Version -ne 1 -or
        $record.Phase -notin @('preparing', 'prepared', 'launching', 'child')) {
        throw 'Invalid launch journal preserved'
    }
    if ($record.ArtifactHash -and $record.ArtifactHash -cnotmatch '^[A-F0-9]{64}$') { throw 'Invalid launch artifact hash preserved' }
    if ($record.Phase -eq 'preparing' -and (-not $record.ArtifactHash -or $null -eq $record.Fence)) { throw 'Incomplete preparing journal preserved' }
    if ($record.ClaimPath) {
        $claim = [IO.Path]::GetFullPath($record.ClaimPath)
        if ([IO.Path]::GetDirectoryName($claim) -cne $RepoRoot -or
            [IO.Path]::GetFileName($claim) -notmatch '^\.codex_discord_rust\.restart\.claimed\.\d+\.\d+$') {
            throw 'Launch claim path is outside the owned restart namespace'
        }
    }
    return $record
}

function Save-CdrLaunchJournal {
    param([string]$Path, $Record)
    Write-AtomicRestartMarker -Path $Path -Text ($Record | ConvertTo-Json -Depth 8 -Compress)
}

function New-CdrLaunchJournal {
    param([string]$Path, [string]$Operation, $Fence = $null, [string]$ArtifactHash,
        $Artifact=$null, [ValidateSet('preparing','prepared')][string]$Phase='prepared')
    $record = Read-CdrLaunchJournal $Path
    if ($null -ne $record) {
        if ($record.Operation -cne $Operation -or
            ($ArtifactHash -and $record.ArtifactHash -cne $ArtifactHash) -or
            ($null -ne $Fence -and -not (Test-RestartDrainFenceMatch $record.Fence $Fence))) {
            throw 'Foreign launch journal preserved'
        }
        return $record
    }
    $record = [pscustomobject]@{
        Version=1; BinaryPath=$BinaryPath; Operation=$Operation; Fence=$Fence
        Phase=$Phase; ChildIdentity=''; ClaimPath=''
        ArtifactHash=$ArtifactHash
    }
    if ($null -ne $Artifact) {
        $record | Add-Member NoteProperty BuildInfo $Artifact.BuildInfo
        $record | Add-Member NoteProperty RequireStartupRecord $Artifact.RequireStartupRecord
    }
    Save-CdrLaunchJournal $Path $record
    return $record
}

function Assert-CdrNoUncertainLegacyBotLaunchJournal {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [string]$IgnoreJournalPath = ''
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $ignoreFull = if (-not [string]::IsNullOrWhiteSpace($IgnoreJournalPath)) { [IO.Path]::GetFullPath($IgnoreJournalPath) } else { '' }
    foreach ($legacyName in @('.codex_discord_rust.restart.launch', '.codex_discord_rust.force.launch', '.codex_discord_rust.launch.journal')) {
        $legacyPath = [IO.Path]::GetFullPath((Join-Path $fullRoot $legacyName))
        if ($ignoreFull -and [StringComparer]::OrdinalIgnoreCase.Equals($legacyPath, $ignoreFull)) {
            continue
        }
        if ([IO.File]::Exists($legacyPath) -or [IO.Directory]::Exists($legacyPath)) {
            $legacyRec = $null
            try {
                $item = Get-Item -LiteralPath $legacyPath -ErrorAction Stop
                if ($item.PSIsContainer) { throw 'legacy_journal_is_directory' }
                $raw = [IO.File]::ReadAllText($legacyPath, [Text.UTF8Encoding]::UTF8)
                if ([string]::IsNullOrWhiteSpace($raw)) { throw 'legacy_journal_empty' }
                $legacyRec = $raw | ConvertFrom-Json -ErrorAction Stop
            } catch {
                throw "launch_outcome_unknown: legacy launch journal $legacyName is corrupt or unreadable ($($_.Exception.Message)); duplicate launch refused"
            }
            if ($null -eq $legacyRec -or
                -not $legacyRec.PSObject.Properties['Version'] -or [int]$legacyRec.Version -ne 1 -or
                -not $legacyRec.PSObject.Properties['Phase'] -or [string]$legacyRec.Phase -notin @('preparing', 'prepared', 'launching', 'child')) {
                throw "launch_outcome_unknown: legacy launch journal $legacyName has invalid schema or phase; duplicate launch refused"
            }
            if ([string]$legacyRec.Phase -in @('preparing', 'prepared', 'launching')) {
                throw "launch_outcome_unknown: $legacyName is in $($legacyRec.Phase) phase; duplicate launch refused"
            }
            if ([string]$legacyRec.Phase -eq 'child') {
                $childId = if ($legacyRec.PSObject.Properties['ChildIdentity']) { [string]$legacyRec.ChildIdentity } else { '' }
                if ($childId -notmatch '^(\d+)\|\d+$') {
                    throw "launch_outcome_unknown: $legacyName in child phase is missing valid ChildIdentity; duplicate launch refused"
                }
                $childPid = [int]$Matches[1]
                $childBin = if ($legacyRec.PSObject.Properties['BinaryPath'] -and -not [string]::IsNullOrWhiteSpace($legacyRec.BinaryPath)) { [string]$legacyRec.BinaryPath } else { '' }
                $childObs = Get-CdrProcessObservation -ProcessId $childPid -ExpectedIdentity $childId -ExpectedBinaryPath $childBin
                if ($childObs.State -eq 'running') {
                    throw "cdr_already_running: bot process $childId from $legacyName is already alive; duplicate launch refused"
                }
                if ($childObs.State -eq 'unknown') {
                    throw "launch_outcome_unknown: bot process $childId from $legacyName observation is unknown ($($childObs.Reason)); duplicate launch refused"
                }
            }
        }
    }
}

if (-not (Get-Variable -Name CdrLaunchingBotIntentRevision -Scope Script -ErrorAction SilentlyContinue)) {
    $script:CdrLaunchingBotIntentRevision = [uint64]0
}

function Set-CdrLaunchStarting {
    $isMaintenanceLaunch = $false
    $record = $null
    if ($script:CdrLaunchJournalPath) {
        $record = Read-CdrLaunchJournal $script:CdrLaunchJournalPath
        if ($record.Phase -ne 'prepared') { throw 'Launch already attempted; refusing duplicate start' }
        $isMaintenanceLaunch = ($record.Operation -like 'maintenance:*' -or $record.Operation -like 'deployment:*')
    }
    $callerRepoRoot = Get-Variable -Name RepoRoot -ValueOnly -ErrorAction SilentlyContinue
    $callerBinaryPath = [string](Get-Variable -Name BinaryPath -ValueOnly -ErrorAction SilentlyContinue)
    $callerLaunchArtifact = Get-Variable -Name launchArtifact -ValueOnly -ErrorAction SilentlyContinue
    if ($callerRepoRoot) {
        Assert-CdrNoUncertainLegacyBotLaunchJournal -RepoRoot $callerRepoRoot -IgnoreJournalPath ([string]$script:CdrLaunchJournalPath)
    }
    if ($callerRepoRoot -and (Get-Command Read-CdrComponentLaunchJournal -ErrorAction SilentlyContinue)) {
        $existingCompJournal = Read-CdrComponentLaunchJournal -RepoRoot $callerRepoRoot -Component 'bot'
        if ($null -ne $existingCompJournal) {
            if ($existingCompJournal.Phase -eq 'launching') {
                Write-CdrComponentLaunchJournal -RepoRoot $callerRepoRoot -Component 'bot' `
                    -Phase 'launch_outcome_unknown' -BinaryPath $callerBinaryPath -Action $existingCompJournal.Action `
                    -IntentRevision ([uint64]$existingCompJournal.IntentRevision) `
                    -Error 'Previous bot launch crashed in launching phase before recording child identity' | Out-Null
                throw 'launch_outcome_unknown: previous bot launch crashed before recording child identity; duplicate launch refused'
            }
            if ($existingCompJournal.Phase -eq 'launch_outcome_unknown') {
                throw 'launch_outcome_unknown: uncertain bot spawn hold active; duplicate launch refused'
            }
            if ($existingCompJournal.Phase -eq 'launched' -and $existingCompJournal.ProcessId -gt 0 -and $existingCompJournal.ProcessIdentity) {
                $obs = Get-CdrProcessObservation -ProcessId ([int]$existingCompJournal.ProcessId) -ExpectedIdentity ([string]$existingCompJournal.ProcessIdentity) -ExpectedBinaryPath $callerBinaryPath
                if ($obs.State -eq 'running') {
                    throw "cdr_already_running: bot process $($existingCompJournal.ProcessIdentity) is already alive; duplicate launch refused"
                }
                if ($obs.State -eq 'unknown') {
                    throw "launch_outcome_unknown: existing bot process $($existingCompJournal.ProcessIdentity) observation is unknown ($($obs.Reason)); duplicate launch refused"
                }
            }
        }
    }
    $botIntentRev = [uint64]0
    if ($callerRepoRoot -and (Get-Command Get-CdrComponentIntent -ErrorAction SilentlyContinue)) {
        $botIntent = Get-CdrComponentIntent -RepoRoot $callerRepoRoot -Component 'bot'
        $botIntentRev = [uint64]$botIntent.Revision
        if ($isMaintenanceLaunch) {
            $botIntentPath = Get-CdrComponentIntentPath -RepoRoot $callerRepoRoot -Component 'bot'
            $exitAllPath = Get-CdrExitAllDecisionPath -RepoRoot $callerRepoRoot
            if (([IO.File]::Exists($botIntentPath) -or [IO.File]::Exists($exitAllPath)) -and $botIntent.DesiredState -eq 'stopped') {
                throw "cdr_manual_stop_active: bot desired state is stopped (revision=$botIntentRev); launch refused"
            }
        } elseif ($botIntent.DesiredState -eq 'stopped' -or (Test-CdrManualStopRequested -RepoRoot $callerRepoRoot -Component 'bot')) {
            throw "cdr_manual_stop_active: bot manual stop intent is active (revision=$botIntentRev); launch refused"
        }
    }
    $script:CdrLaunchingBotIntentRevision = $botIntentRev
    $compArtHash = if ($null -ne $callerLaunchArtifact -and $callerLaunchArtifact.PSObject.Properties['ArtifactHash']) { [string]$callerLaunchArtifact.ArtifactHash } else { '' }
    $compBuildInfo = if ($null -ne $callerLaunchArtifact -and $callerLaunchArtifact.PSObject.Properties['BuildInfo']) { $callerLaunchArtifact.BuildInfo } else { $null }
    $compReqStartup = [bool]($null -ne $callerLaunchArtifact -and $callerLaunchArtifact.PSObject.Properties['RequireStartupRecord'] -and $callerLaunchArtifact.RequireStartupRecord)
    if ($callerRepoRoot -and (Get-Command Write-CdrComponentLaunchJournal -ErrorAction SilentlyContinue)) {
        Write-CdrComponentLaunchJournal -RepoRoot $callerRepoRoot -Component 'bot' -Phase 'launching' `
            -BinaryPath $callerBinaryPath -Action 'watchdog-start' -IntentRevision $botIntentRev `
            -ArtifactHash $compArtHash -BuildInfo $compBuildInfo -RequireStartupRecord:$compReqStartup | Out-Null
    }
    if (-not $script:CdrLaunchJournalPath) { return }
    $record.Phase = 'launching'
    Save-CdrLaunchJournal $script:CdrLaunchJournalPath $record
}

function Set-CdrLaunchChild {
    param($Process)
    $identity = if (Get-Command Get-RustProcessIdentity -ErrorAction SilentlyContinue) {
        Get-RustProcessIdentity $Process
    } else {
        Get-CdrProcessIdentity -Process $Process
    }
    if ($identity -notmatch '^\d+\|\d+$') {
        throw 'Launch child identity cannot be durably bound'
    }
    $callerRepoRoot = Get-Variable -Name RepoRoot -ValueOnly -ErrorAction SilentlyContinue
    $callerBinaryPath = [string](Get-Variable -Name BinaryPath -ValueOnly -ErrorAction SilentlyContinue)
    $callerLaunchArtifact = Get-Variable -Name launchArtifact -ValueOnly -ErrorAction SilentlyContinue
    $rev = if ($null -ne $script:CdrLaunchingBotIntentRevision) { [uint64]$script:CdrLaunchingBotIntentRevision } else { [uint64]0 }
    $compArtHash = if ($null -ne $callerLaunchArtifact -and $callerLaunchArtifact.PSObject.Properties['ArtifactHash']) { [string]$callerLaunchArtifact.ArtifactHash } else { '' }
    $compBuildInfo = if ($null -ne $callerLaunchArtifact -and $callerLaunchArtifact.PSObject.Properties['BuildInfo']) { $callerLaunchArtifact.BuildInfo } else { $null }
    $compReqStartup = [bool]($null -ne $callerLaunchArtifact -and $callerLaunchArtifact.PSObject.Properties['RequireStartupRecord'] -and $callerLaunchArtifact.RequireStartupRecord)
    if ($callerRepoRoot -and (Get-Command Write-CdrComponentLaunchJournal -ErrorAction SilentlyContinue)) {
        Write-CdrComponentLaunchJournal -RepoRoot $callerRepoRoot -Component 'bot' -Phase 'launched' -BinaryPath $callerBinaryPath `
            -Action 'watchdog-start' -IntentRevision $rev -ProcessId ([int]$Process.Id) -ProcessIdentity $identity `
            -ArtifactHash $compArtHash -BuildInfo $compBuildInfo -RequireStartupRecord:$compReqStartup | Out-Null
    }
    if (-not $script:CdrLaunchJournalPath) { return }
    $record = Read-CdrLaunchJournal $script:CdrLaunchJournalPath
    if ($record.Phase -ne 'launching') {
        throw 'Launch child identity cannot be durably bound'
    }
    $record.ChildIdentity = $identity
    $record.Phase = 'child'
    Save-CdrLaunchJournal $script:CdrLaunchJournalPath $record
}

function Get-CdrRecordedChild {
    param($Record)
    if ($Record.Phase -eq 'launching') {
        throw 'launch_outcome_unknown: child creation was not durably recorded; no automatic duplicate launch'
    }
    if ($Record.Phase -eq 'preparing') { throw 'Restart preparation is incomplete; launch is not authorized' }
    if ($Record.Phase -eq 'prepared') { return $null }
    if ($Record.ChildIdentity -notmatch '^(\d+)\|\d+$') { throw 'Invalid recorded child identity' }
    $childPid = [int]$Matches[1]
    $process = Get-Process -Id $childPid -ErrorAction SilentlyContinue
    if ($null -eq $process) { return $null }
    if ($process.Path -cne $BinaryPath -or
        (Get-RustProcessIdentity $process) -cne $Record.ChildIdentity) {
        throw 'Recorded child PID now belongs to another instance; preserved'
    }
    return $process
}

function Write-CdrAtomicJsonFile {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)]$Object
    )
    $parent = Split-Path -Parent $Path
    if ($parent -and -not [IO.Directory]::Exists($parent)) {
        [IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    $text = $Object | ConvertTo-Json -Depth 10 -Compress
    $temp = "$Path.tmp.$PID.$([guid]::NewGuid().ToString('N'))"
    try {
        [IO.File]::WriteAllText($temp, $text, [Text.UTF8Encoding]::new($false))
        if ([IO.File]::Exists($Path)) {
            [IO.File]::Replace($temp, $Path, [NullString]::Value)
        } else {
            [IO.File]::Move($temp, $Path)
        }
    } finally {
        if ([IO.File]::Exists($temp)) {
            [IO.File]::Delete($temp)
        }
    }
}

if (-not (Get-Command Get-CdrProcessIdentity -ErrorAction SilentlyContinue)) {
    function Get-CdrProcessIdentity {
        param($Process = $null, [int]$ProcessId = 0)
        $target = $Process
        if ($null -eq $target -and $ProcessId -gt 0) {
            $target = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        }
        if ($null -eq $target) { return $null }
        try {
            $ticks = $target.StartTime.ToUniversalTime().Ticks
            if ($ticks -le 0) { return $null }
            $ticks -= ($ticks % 10)
            return "$($target.Id)|$ticks"
        } catch {
            return $null
        }
    }
}

if (-not (Get-Command Get-CdrProcessObservation -ErrorAction SilentlyContinue)) {
    function Get-CdrProcessObservation {
        param(
            [Parameter(Mandatory)][int]$ProcessId,
            [Parameter(Mandatory)][string]$ExpectedIdentity,
            [Alias('ExpectedPath')][string]$ExpectedBinaryPath = '',
            $Process = $null
        )
        if ($ProcessId -le 0 -or $ExpectedIdentity -notmatch '^(\d+)\|(\d+)$') {
            return [pscustomobject]@{ State = 'unknown'; Matches = $false; Process = $null; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'invalid_expected_identity' }
        }
        if ([int]$Matches[1] -ne $ProcessId) {
            return [pscustomobject]@{ State = 'unknown'; Matches = $false; Process = $null; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'pid_mismatch' }
        }
        $proc = $Process
        try {
            if ($null -eq $proc) {
                $proc = Get-Process -Id $ProcessId -ErrorAction Stop
            } elseif ($proc.PSObject.Methods['Refresh']) {
                try { $proc.Refresh() } catch { }
            }
        } catch {
            if ($_.FullyQualifiedErrorId -like 'NoProcessFoundForGivenId*' -or
                $_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound -or
                $_.Exception.Message -match 'Cannot find a process with the process identifier') {
                return [pscustomobject]@{ State = 'stopped'; Matches = $false; Process = $null; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'not_found' }
            }
            return [pscustomobject]@{ State = 'unknown'; Matches = $false; Process = $null; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = "get_process_failed:$($_.Exception.GetType().FullName)" }
        }
        if ($null -eq $proc) {
            return [pscustomobject]@{ State = 'stopped'; Matches = $false; Process = $null; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'null_process' }
        }
        try {
            try { [void]$proc.Handle } catch { }
            if ($proc.PSObject.Properties['HasExited'] -and $proc.HasExited) {
                return [pscustomobject]@{ State = 'stopped'; Matches = $false; Process = $null; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'has_exited' }
            }
            $actualIdentity = Get-CdrProcessIdentity -Process $proc
            if ([string]::IsNullOrWhiteSpace($actualIdentity)) {
                return [pscustomobject]@{ State = 'unknown'; Matches = $false; Process = $proc; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'start_time_unavailable' }
            }
            if ($actualIdentity -cne $ExpectedIdentity) {
                return [pscustomobject]@{ State = 'stopped'; Matches = $false; Process = $null; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'identity_reused' }
            }
            $resolvedBinary = $ExpectedBinaryPath
            if (-not [string]::IsNullOrWhiteSpace($ExpectedBinaryPath)) {
                $actualPath = $proc.Path
                if ([string]::IsNullOrWhiteSpace($actualPath) -and $proc.PSObject.Properties['MainModule'] -and $null -ne $proc.MainModule) {
                    $actualPath = $proc.MainModule.FileName
                }
                if ([string]::IsNullOrWhiteSpace($actualPath)) {
                    return [pscustomobject]@{ State = 'unknown'; Matches = $false; Process = $proc; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'path_unavailable' }
                }
                if (-not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($actualPath), [IO.Path]::GetFullPath($ExpectedBinaryPath))) {
                    return [pscustomobject]@{ State = 'unknown'; Matches = $false; Process = $proc; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = 'path_mismatch' }
                }
                $resolvedBinary = [IO.Path]::GetFullPath($actualPath)
            }
            return [pscustomobject]@{ State = 'running'; Matches = $true; Process = $proc; ProcessId = [int]$ProcessId; ProcessIdentity = $actualIdentity; BinaryPath = $resolvedBinary; Reason = 'matched' }
        } catch {
            return [pscustomobject]@{ State = 'unknown'; Matches = $false; Process = $proc; ProcessId = $ProcessId; ProcessIdentity = $ExpectedIdentity; BinaryPath = $ExpectedBinaryPath; Reason = "observation_exception:$($_.Exception.GetType().FullName)" }
        }
    }
}

if (-not (Get-Command Test-CdrProcessMatchesIdentity -ErrorAction SilentlyContinue)) {
    function Test-CdrProcessMatchesIdentity {
        param(
            [Parameter(Mandatory)][int]$ProcessId,
            [Parameter(Mandatory)][string]$ExpectedIdentity,
            [Alias('ExpectedPath')][string]$ExpectedBinaryPath = '',
            $Process = $null
        )
        return [bool](Get-CdrProcessObservation -ProcessId $ProcessId -ExpectedIdentity $ExpectedIdentity -ExpectedBinaryPath $ExpectedBinaryPath -Process $Process).Matches
    }
}

function Stop-CdrVerifiedProcess {
    param(
        [Parameter(Mandatory)][int]$ProcessId,
        [Parameter(Mandatory)][string]$ExpectedIdentity,
        [string]$ExpectedBinaryPath = '',
        $Process = $null,
        [int]$WaitTimeoutMs = 5000
    )
    $obs = Get-CdrProcessObservation -ProcessId $ProcessId -ExpectedIdentity $ExpectedIdentity -ExpectedBinaryPath $ExpectedBinaryPath -Process $Process
    if (-not $obs.Matches -or $obs.State -ne 'running' -or $null -eq $obs.Process) {
        return $false
    }
    $targetProc = $obs.Process
    $disposableLocal = ($null -eq $Process)
    try {
        [void]$targetProc.Handle
        if ($targetProc.PSObject.Properties['HasExited'] -and $targetProc.HasExited) {
            return $true
        }
        if ((Get-CdrProcessIdentity -Process $targetProc) -cne $ExpectedIdentity) {
            return $false
        }
        if (-not [string]::IsNullOrWhiteSpace($ExpectedBinaryPath)) {
            $actualPath = $targetProc.Path
            if ([string]::IsNullOrWhiteSpace($actualPath) -and $targetProc.PSObject.Properties['MainModule'] -and $null -ne $targetProc.MainModule) {
                $actualPath = $targetProc.MainModule.FileName
            }
            if ([string]::IsNullOrWhiteSpace($actualPath) -or
                -not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($actualPath), [IO.Path]::GetFullPath($ExpectedBinaryPath))) {
                return $false
            }
        }
        try {
            $targetProc.Kill()
        } catch {
            if (-not ($targetProc.PSObject.Properties['HasExited'] -and $targetProc.HasExited)) {
                return $false
            }
        }
        if ($WaitTimeoutMs -gt 0 -and ($targetProc -is [Diagnostics.Process] -or $null -ne $targetProc.PSObject.Methods['WaitForExit'])) {
            if (-not $targetProc.WaitForExit($WaitTimeoutMs)) {
                return $false
            }
        }
        if ($targetProc -is [Diagnostics.Process] -or $null -ne $targetProc.PSObject.Methods['Refresh']) {
            try { $targetProc.Refresh() } catch { }
        }
        return [bool]($targetProc.PSObject.Properties['HasExited'] -and $targetProc.HasExited)
    } catch {
        return $false
    } finally {
        if ($disposableLocal -and $targetProc -is [IDisposable]) {
            $targetProc.Dispose()
        }
    }
}

function Read-CdrMergedEnvMap {
    param([Parameter(Mandatory)][Alias('Root')][string]$RepoRoot)
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $map = @{}
    foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
        $map[[string]$entry.Key] = [string]$entry.Value
    }
    $envPath = Join-Path $fullRoot '.env'
    if ([IO.File]::Exists($envPath)) {
        foreach ($rawLine in [IO.File]::ReadAllLines($envPath, [Text.UTF8Encoding]::UTF8)) {
            $line = $rawLine.Trim()
            if ([string]::IsNullOrWhiteSpace($line) -or $line.StartsWith('#')) { continue }
            $idx = $line.IndexOf('=')
            if ($idx -le 0) { continue }
            $k = $line.Substring(0, $idx).Trim()
            $v = $line.Substring($idx + 1).Trim()
            if ($v.Length -ge 2 -and (($v.StartsWith('"') -and $v.EndsWith('"')) -or ($v.StartsWith("'") -and $v.EndsWith("'")))) {
                $v = $v.Substring(1, $v.Length - 2)
            }
            if (-not $map.ContainsKey($k)) {
                $map[$k] = $v
            }
        }
    }
    return $map
}

function Resolve-CdrRemoteMcpPaths {
    param([Parameter(Mandatory)][Alias('Root')][string]$RepoRoot)
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $envMap = Read-CdrMergedEnvMap -RepoRoot $fullRoot

    $dataDirRel = if ($envMap.ContainsKey('CODEX_MCP_DATA_DIR') -and -not [string]::IsNullOrWhiteSpace($envMap['CODEX_MCP_DATA_DIR'])) {
        $envMap['CODEX_MCP_DATA_DIR'].Trim()
    } elseif ($envMap.ContainsKey('CODEX_BOT_DATA_DIR') -and -not [string]::IsNullOrWhiteSpace($envMap['CODEX_BOT_DATA_DIR'])) {
        $envMap['CODEX_BOT_DATA_DIR'].Trim()
    } else {
        'bot_data'
    }
    $dataDir = if ([IO.Path]::IsPathRooted($dataDirRel)) {
        [IO.Path]::GetFullPath($dataDirRel)
    } else {
        [IO.Path]::GetFullPath((Join-Path $fullRoot $dataDirRel))
    }

    $statusPath = if ($envMap.ContainsKey('CODEX_REMOTE_MCP_STATUS_FILE') -and -not [string]::IsNullOrWhiteSpace($envMap['CODEX_REMOTE_MCP_STATUS_FILE'])) {
        $sRel = $envMap['CODEX_REMOTE_MCP_STATUS_FILE'].Trim()
        if ([IO.Path]::IsPathRooted($sRel)) { [IO.Path]::GetFullPath($sRel) } else { [IO.Path]::GetFullPath((Join-Path $fullRoot $sRel)) }
    } else {
        Join-Path $dataDir 'remote_mcp_status.json'
    }

    $stopPath = if ($envMap.ContainsKey('CODEX_REMOTE_MCP_STOP_FILE') -and -not [string]::IsNullOrWhiteSpace($envMap['CODEX_REMOTE_MCP_STOP_FILE'])) {
        $stRel = $envMap['CODEX_REMOTE_MCP_STOP_FILE'].Trim()
        if ([IO.Path]::IsPathRooted($stRel)) { [IO.Path]::GetFullPath($stRel) } else { [IO.Path]::GetFullPath((Join-Path $fullRoot $stRel)) }
    } else {
        Join-Path $dataDir 'remote_mcp.stop'
    }

    $instanceLockPath = Join-Path $dataDir 'remote_mcp.instance.lock'
    $enabledRaw = if ($envMap.ContainsKey('CODEX_REMOTE_MCP_ENABLED')) { $envMap['CODEX_REMOTE_MCP_ENABLED'].Trim().ToLowerInvariant() } else { '' }
    $enabled = ($enabledRaw -in @('1', 'true', 'yes', 'on'))

    $configuredOwnership = if ($envMap.ContainsKey('CODEX_REMOTE_MCP_OWNERSHIP') -and -not [string]::IsNullOrWhiteSpace($envMap['CODEX_REMOTE_MCP_OWNERSHIP'])) {
        $envMap['CODEX_REMOTE_MCP_OWNERSHIP'].Trim().ToLowerInvariant()
    } else {
        'embedded'
    }

    $ownershipStatePath = Join-Path $fullRoot '.cdr-maintenance\mcp-ownership-state.json'
    $effectiveOwnership = $configuredOwnership
    $ownershipCommitted = ($configuredOwnership -eq 'external')
    if ([IO.File]::Exists($ownershipStatePath)) {
        try {
            $stateJson = [IO.File]::ReadAllText($ownershipStatePath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json
            $stateSchema = if ($stateJson.PSObject.Properties['schema_version']) { [int]$stateJson.schema_version } else { 0 }
            $stateMode = if ($stateJson.PSObject.Properties['mode']) { [string]$stateJson.mode } else { '' }
            $hasState = [bool]$stateJson.PSObject.Properties['state']
            $hasCommitted = [bool]$stateJson.PSObject.Properties['committed']
            $committedBool = if ($hasState -and $hasCommitted) {
                ([string]$stateJson.state -eq 'committed') -and ([bool]$stateJson.committed)
            } elseif ($hasState) {
                ([string]$stateJson.state -eq 'committed')
            } elseif ($hasCommitted) {
                ([bool]$stateJson.committed)
            } else {
                $false
            }
            if ($stateSchema -eq 1 -and $stateMode -eq 'embedded' -and $committedBool) {
                $effectiveOwnership = 'embedded'
                $ownershipCommitted = $true
            } else {
                $effectiveOwnership = 'external'
                $ownershipCommitted = ($stateSchema -eq 1 -and $stateMode -eq 'external' -and $committedBool)
            }
        } catch {
            $effectiveOwnership = 'external'
            $ownershipCommitted = $false
        }
    }

    $deviceId = if ($envMap.ContainsKey('CODEX_REMOTE_MCP_DEVICE_ID')) { $envMap['CODEX_REMOTE_MCP_DEVICE_ID'].Trim() } else { '' }
    $bridgeUrl = if ($envMap.ContainsKey('CODEX_REMOTE_MCP_BRIDGE_URL')) { $envMap['CODEX_REMOTE_MCP_BRIDGE_URL'].Trim() } else { '' }
    $ttl = 1800
    if ($envMap.ContainsKey('CODEX_REMOTE_MCP_BINDING_TTL_SECONDS')) {
        $parsedTtl = 0
        if ([int]::TryParse($envMap['CODEX_REMOTE_MCP_BINDING_TTL_SECONDS'].Trim(), [ref]$parsedTtl) -and $parsedTtl -gt 0) {
            $ttl = $parsedTtl
        }
    }
    $bridgeFingerprint = ''
    if (-not [string]::IsNullOrWhiteSpace($bridgeUrl) -and -not [string]::IsNullOrWhiteSpace($deviceId)) {
        try {
            $normalizedUrl = ([Uri]::new($bridgeUrl)).AbsoluteUri
            $fpInput = "v1|url=$normalizedUrl|device_id=$deviceId|ttl=$ttl"
            $sha = [Security.Cryptography.SHA256]::Create()
            try {
                $bytes = [Text.Encoding]::UTF8.GetBytes($fpInput)
                $bridgeFingerprint = [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant()
            } finally {
                $sha.Dispose()
            }
        } catch { }
    }

    return [pscustomobject]@{
        InstallRoot         = $fullRoot
        NormalizedRoot      = $fullRoot.TrimEnd('\').Replace('\', '/').ToLowerInvariant()
        DataDir             = $dataDir
        StatusPath          = $statusPath
        StopPath            = $stopPath
        InstanceLockPath    = $instanceLockPath
        Enabled             = $enabled
        ConfiguredOwnership = $configuredOwnership
        EffectiveOwnership  = $effectiveOwnership
        OwnershipCommitted  = $ownershipCommitted
        DeviceId            = $deviceId
        BridgeUrl           = $bridgeUrl
        BridgeFingerprint   = $bridgeFingerprint
    }
}

function Get-CdrComponentIntentPath {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot'
    )
    return Join-Path ([IO.Path]::GetFullPath($RepoRoot)) ".cdr-maintenance\$Component-intent.json"
}

function Get-CdrComponentLaunchJournalPath {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot'
    )
    $fileName = if ($Component -eq 'mcp') { 'mcp-launch-journal.json' } else { 'runtime-launch-journal.json' }
    return Join-Path ([IO.Path]::GetFullPath($RepoRoot)) ".cdr-maintenance\$fileName"
}

$script:CdrIntentLockHandle = $null
$script:CdrIntentLockRoot = ''
$script:CdrIntentLockDepth = 0

function Enter-CdrIntentLock {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [int]$TimeoutMs = 15000
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    if ($null -ne $script:CdrIntentLockHandle -and $script:CdrIntentLockDepth -gt 0 -and
        [StringComparer]::OrdinalIgnoreCase.Equals($script:CdrIntentLockRoot, $fullRoot)) {
        $script:CdrIntentLockDepth++
        return [pscustomobject]@{ IsReentrant = $true }
    }
    $maintDir = Join-Path $fullRoot '.cdr-maintenance'
    if (-not [IO.Directory]::Exists($maintDir)) {
        [IO.Directory]::CreateDirectory($maintDir) | Out-Null
    }
    $lockPath = Join-Path $maintDir 'intent.lock'
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ($true) {
        try {
            $handle = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
            $script:CdrIntentLockHandle = $handle
            $script:CdrIntentLockRoot = $fullRoot
            $script:CdrIntentLockDepth = 1
            return [pscustomobject]@{ IsReentrant = $false; Handle = $handle }
        } catch [IO.IOException] {
            if ([DateTime]::UtcNow -ge $deadline) {
                throw "cdr_control_busy: intent lock at $lockPath is busy"
            }
            Start-Sleep -Milliseconds 15
        }
    }
}

function Exit-CdrIntentLock {
    param($LockToken)
    if ($null -eq $LockToken) { return }
    if ($LockToken.PSObject.Properties['IsReentrant'] -and $LockToken.IsReentrant) {
        if ($script:CdrIntentLockDepth -gt 1) {
            $script:CdrIntentLockDepth--
        }
        return
    }
    $handle = if ($LockToken.PSObject.Properties['Handle']) { $LockToken.Handle } else { $LockToken }
    $script:CdrIntentLockHandle = $null
    $script:CdrIntentLockRoot = ''
    $script:CdrIntentLockDepth = 0
    if ($null -ne $handle -and $handle -is [IDisposable]) {
        $handle.Dispose()
    }
}

$script:CdrShortLockHandle = $null
$script:CdrShortLockKey = ''
$script:CdrShortLockDepth = 0

function Enter-CdrComponentShortLock {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp', 'common')][string]$Component = 'common',
        [int]$TimeoutMs = 15000
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $key = "$($fullRoot.ToLowerInvariant())|$Component"
    if ($null -ne $script:CdrShortLockHandle -and $script:CdrShortLockDepth -gt 0 -and
        [StringComparer]::OrdinalIgnoreCase.Equals($script:CdrShortLockKey, $key)) {
        $script:CdrShortLockDepth++
        return [pscustomobject]@{ IsReentrant = $true }
    }
    $maintDir = Join-Path $fullRoot '.cdr-maintenance'
    if (-not [IO.Directory]::Exists($maintDir)) {
        [IO.Directory]::CreateDirectory($maintDir) | Out-Null
    }
    $lockPath = Join-Path $maintDir "$Component-launch.lock"
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ($true) {
        try {
            $handle = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
            $script:CdrShortLockHandle = $handle
            $script:CdrShortLockKey = $key
            $script:CdrShortLockDepth = 1
            return [pscustomobject]@{ IsReentrant = $false; Handle = $handle }
        } catch [IO.IOException] {
            if ([DateTime]::UtcNow -ge $deadline) {
                throw "cdr_control_busy: $Component launch lock at $lockPath is busy"
            }
            Start-Sleep -Milliseconds 15
        }
    }
}

function Exit-CdrComponentShortLock {
    param($LockToken)
    if ($null -eq $LockToken) { return }
    if ($LockToken.PSObject.Properties['IsReentrant'] -and $LockToken.IsReentrant) {
        if ($script:CdrShortLockDepth -gt 1) {
            $script:CdrShortLockDepth--
        }
        return
    }
    $handle = if ($LockToken.PSObject.Properties['Handle']) { $LockToken.Handle } else { $LockToken }
    $script:CdrShortLockHandle = $null
    $script:CdrShortLockKey = ''
    $script:CdrShortLockDepth = 0
    if ($null -ne $handle -and $handle -is [IDisposable]) {
        $handle.Dispose()
    }
}

function Set-CdrManualStopMarker {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [string]$Reason = 'manual_stop'
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    if ($Component -eq 'bot') {
        $stopFile = Join-Path $fullRoot '.codex_discord_rust.stop'
        $disabledFile = Join-Path $fullRoot '.codex_discord_bot.disabled'
        [IO.File]::WriteAllText($stopFile, $Reason, [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($disabledFile, 'disabled_by_user_stop', [Text.UTF8Encoding]::new($false))
    } else {
        $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
        $parent = Split-Path -Parent $mcpPaths.StopPath
        if ($parent -and -not [IO.Directory]::Exists($parent)) {
            [IO.Directory]::CreateDirectory($parent) | Out-Null
        }
        [IO.File]::WriteAllText($mcpPaths.StopPath, 'stop', [Text.UTF8Encoding]::new($false))
    }
}

function Clear-CdrManualStopMarker {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot'
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    if ($Component -eq 'bot') {
        foreach ($name in @('.codex_discord_rust.stop', '.codex_discord_bot.disabled')) {
            $p = Join-Path $fullRoot $name
            if ([IO.File]::Exists($p)) { [IO.File]::Delete($p) }
        }
    } else {
        $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
        if ([IO.File]::Exists($mcpPaths.StopPath)) { [IO.File]::Delete($mcpPaths.StopPath) }
        $defaultStop = Join-Path $fullRoot 'bot_data\remote_mcp.stop'
        if ([IO.File]::Exists($defaultStop)) { [IO.File]::Delete($defaultStop) }
    }
}

function Get-CdrExitAllDecisionPath {
    param([Parameter(Mandatory)][Alias('Root')][string]$RepoRoot)
    return Join-Path ([IO.Path]::GetFullPath($RepoRoot)) '.cdr-maintenance\exit-all-intent.json'
}

function Read-CdrRawComponentIntentFile {
    param([Parameter(Mandatory)][string]$Path)
    if (-not [IO.File]::Exists($Path)) { return $null }
    try {
        $record = [IO.File]::ReadAllText($Path, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
        return [pscustomobject]@{
            SchemaVersion = [int]$record.SchemaVersion
            Component     = [string]$record.Component
            Revision      = [uint64]$record.Revision
            DesiredState  = [string]$record.DesiredState
            Action        = [string]$record.Action
            Source        = [string]$record.Source
            UpdatedAtMs   = [int64]$record.UpdatedAtMs
        }
    } catch {
        return $null
    }
}

function Resolve-CdrPendingExitAllDecision {
    param([Parameter(Mandatory)][Alias('Root')][string]$RepoRoot)
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $exitAllPath = Get-CdrExitAllDecisionPath -RepoRoot $fullRoot
    if (-not [IO.File]::Exists($exitAllPath)) { return }
    $decision = $null
    try {
        $decision = [IO.File]::ReadAllText($exitAllPath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
    } catch {
        return
    }
    if ($null -eq $decision -or [int]$decision.SchemaVersion -ne 1) { return }
    if ([string]$decision.State -ne 'pending_signals') { return }

    $lock = Enter-CdrIntentLock -RepoRoot $fullRoot
    try {
        $decision = [IO.File]::ReadAllText($exitAllPath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
        if ([string]$decision.State -ne 'pending_signals') { return }
        $botRev = [uint64]$decision.BotRevision
        $mcpRev = [uint64]$decision.McpRevision
        $source = if ($decision.PSObject.Properties['Source'] -and -not [string]::IsNullOrWhiteSpace($decision.Source)) { [string]$decision.Source } else { 'tray-exit-all' }
        $nowMs = if ($decision.PSObject.Properties['UpdatedAtMs'] -and [int64]$decision.UpdatedAtMs -gt 0) { [int64]$decision.UpdatedAtMs } else { [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }

        $botPath = Get-CdrComponentIntentPath -RepoRoot $fullRoot -Component 'bot'
        $botCur = Read-CdrRawComponentIntentFile -Path $botPath
        if ($null -eq $botCur -or $botCur.Revision -lt $botRev) {
            Write-CdrAtomicJsonFile -Path $botPath -Object ([ordered]@{
                SchemaVersion = 1; Component = 'bot'; Revision = $botRev
                DesiredState = 'stopped'; Action = 'exit-all'; Source = $source; UpdatedAtMs = $nowMs
            })
        }

        $mcpPath = Get-CdrComponentIntentPath -RepoRoot $fullRoot -Component 'mcp'
        $mcpCur = Read-CdrRawComponentIntentFile -Path $mcpPath
        if ($null -eq $mcpCur -or $mcpCur.Revision -lt $mcpRev) {
            Write-CdrAtomicJsonFile -Path $mcpPath -Object ([ordered]@{
                SchemaVersion = 1; Component = 'mcp'; Revision = $mcpRev
                DesiredState = 'stopped'; Action = 'exit-all'; Source = $source; UpdatedAtMs = $nowMs
            })
        }

        $botFinal = Read-CdrRawComponentIntentFile -Path $botPath
        if ($null -ne $botFinal -and $botFinal.DesiredState -eq 'stopped') {
            Set-CdrManualStopMarker -RepoRoot $fullRoot -Component 'bot' -Reason 'exit-all'
        }
        $mcpFinal = Read-CdrRawComponentIntentFile -Path $mcpPath
        if ($null -ne $mcpFinal -and $mcpFinal.DesiredState -eq 'stopped') {
            Set-CdrManualStopMarker -RepoRoot $fullRoot -Component 'mcp' -Reason 'exit-all'
        }

        Write-CdrAtomicJsonFile -Path $exitAllPath -Object ([ordered]@{
            SchemaVersion = 1
            State         = 'committed'
            DesiredState  = 'stopped'
            Action        = 'exit-all'
            Source        = $source
            BotRevision   = $botRev
            McpRevision   = $mcpRev
            UpdatedAtMs   = $nowMs
        })
    } finally {
        Exit-CdrIntentLock $lock
    }
}

function Get-CdrComponentIntent {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot'
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    Resolve-CdrPendingExitAllDecision -RepoRoot $fullRoot
    $path = Get-CdrComponentIntentPath -RepoRoot $fullRoot -Component $Component
    if ([IO.File]::Exists($path)) {
        $record = [IO.File]::ReadAllText($path, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json
        return [pscustomobject]@{
            SchemaVersion = [int]$record.SchemaVersion
            Component     = [string]$record.Component
            Revision      = [uint64]$record.Revision
            DesiredState  = [string]$record.DesiredState
            Action        = [string]$record.Action
            Source        = [string]$record.Source
            UpdatedAtMs   = [int64]$record.UpdatedAtMs
        }
    }
    $hasStopMarker = $false
    if ($Component -eq 'bot') {
        $hasStopMarker = [IO.File]::Exists((Join-Path $fullRoot '.codex_discord_rust.stop')) -or
            [IO.File]::Exists((Join-Path $fullRoot '.codex_discord_bot.disabled'))
    } else {
        $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
        if ([IO.File]::Exists($mcpPaths.StopPath)) {
            $raw = [IO.File]::ReadAllText($mcpPaths.StopPath).Trim().ToLowerInvariant()
            $hasStopMarker = ($raw -in @('stop', 'manual_stop', 'disabled_by_user_stop'))
        }
    }
    return [pscustomobject]@{
        SchemaVersion = 1
        Component     = $Component
        Revision      = [uint64]0
        DesiredState  = if ($hasStopMarker) { 'stopped' } else { 'running' }
        Action        = 'default'
        Source        = 'default'
        UpdatedAtMs   = [int64]0
    }
}

function Test-CdrManualStopRequested {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot'
    )
    $intent = Get-CdrComponentIntent -RepoRoot $RepoRoot -Component $Component
    if ($intent.DesiredState -eq 'stopped') { return $true }
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    if ($Component -eq 'bot') {
        return [IO.File]::Exists((Join-Path $fullRoot '.codex_discord_rust.stop')) -or
            [IO.File]::Exists((Join-Path $fullRoot '.codex_discord_bot.disabled'))
    } else {
        $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
        if (-not [IO.File]::Exists($mcpPaths.StopPath)) { return $false }
        $raw = [IO.File]::ReadAllText($mcpPaths.StopPath).Trim().ToLowerInvariant()
        return ($raw -in @('stop', 'manual_stop', 'disabled_by_user_stop'))
    }
}

function Set-CdrComponentIntent {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [Parameter(Mandatory)][ValidateSet('running', 'stopped')][string]$DesiredState,
        [string]$Action = 'unspecified',
        [string]$Source = 'control',
        [scriptblock]$OnAfterIntentPersisted = $null
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $lock = Enter-CdrIntentLock -RepoRoot $fullRoot
    try {
        $current = Get-CdrComponentIntent -RepoRoot $fullRoot -Component $Component
        $nextRevision = [uint64]($current.Revision + 1)
        $record = [ordered]@{
            SchemaVersion = 1
            Component     = $Component
            Revision      = $nextRevision
            DesiredState  = $DesiredState
            Action        = $Action
            Source        = $Source
            UpdatedAtMs   = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        }
        # Persist the durable intent/revision JSON FIRST before signaling/clearing stop markers
        $path = Get-CdrComponentIntentPath -RepoRoot $fullRoot -Component $Component
        Write-CdrAtomicJsonFile -Path $path -Object $record
        if ($null -ne $OnAfterIntentPersisted) {
            & $OnAfterIntentPersisted ([pscustomobject]$record)
        }
        if ($DesiredState -eq 'stopped') {
            Set-CdrManualStopMarker -RepoRoot $fullRoot -Component $Component -Reason $Action
        } else {
            Clear-CdrManualStopMarker -RepoRoot $fullRoot -Component $Component
        }
        return [pscustomobject]$record
    } finally {
        Exit-CdrIntentLock $lock
    }
}

function Complete-CdrComponentIntentIfCurrent {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [Parameter(Mandatory)][uint64]$ExpectedRevision,
        [ValidateSet('running', 'stopped')][string]$ResultState = 'running',
        [string]$Source = 'completion'
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $lock = Enter-CdrIntentLock -RepoRoot $fullRoot
    try {
        $current = Get-CdrComponentIntent -RepoRoot $fullRoot -Component $Component
        if ($current.Revision -ne $ExpectedRevision) {
            return $false
        }
        if ($ResultState -eq 'running' -and $current.DesiredState -eq 'stopped') {
            return $false
        }
        $record = [ordered]@{
            SchemaVersion     = 1
            Component         = $Component
            Revision          = [uint64]$current.Revision
            DesiredState      = $ResultState
            Action            = [string]$current.Action
            Source            = [string]$current.Source
            UpdatedAtMs       = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            CompletedRevision = [uint64]$ExpectedRevision
            CompletedBy       = $Source
        }
        $path = Get-CdrComponentIntentPath -RepoRoot $fullRoot -Component $Component
        Write-CdrAtomicJsonFile -Path $path -Object $record
        return $true
    } finally {
        Exit-CdrIntentLock $lock
    }
}

function Set-CdrExitAllIntent {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [string]$Source = 'tray-exit-all',
        [scriptblock]$OnAfterDecisionPersisted = $null,
        [scriptblock]$OnAfterFirstIntentPersisted = $null,
        [scriptblock]$OnAfterBothIntentsPersisted = $null
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $lock = Enter-CdrIntentLock -RepoRoot $fullRoot
    try {
        $botCur = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'bot'
        $mcpCur = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp'
        $nextBotRev = [uint64]($botCur.Revision + 1)
        $nextMcpRev = [uint64]($mcpCur.Revision + 1)
        $nowMs = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()

        $exitAllPath = Get-CdrExitAllDecisionPath -RepoRoot $fullRoot
        $decision = [ordered]@{
            SchemaVersion = 1
            State         = 'pending_signals'
            DesiredState  = 'stopped'
            Action        = 'exit-all'
            Source        = $Source
            BotRevision   = $nextBotRev
            McpRevision   = $nextMcpRev
            UpdatedAtMs   = $nowMs
        }
        # 1. Persist recoverable atomic Exit-All decision before touching component intents or stop markers
        Write-CdrAtomicJsonFile -Path $exitAllPath -Object $decision
        if ($null -ne $OnAfterDecisionPersisted) {
            & $OnAfterDecisionPersisted ([pscustomobject]$decision)
        }

        # 2. Persist BOTH component durable intent JSON records before signaling either runtime
        $botRecord = [ordered]@{
            SchemaVersion = 1; Component = 'bot'; Revision = $nextBotRev
            DesiredState = 'stopped'; Action = 'exit-all'; Source = $Source; UpdatedAtMs = $nowMs
        }
        $mcpRecord = [ordered]@{
            SchemaVersion = 1; Component = 'mcp'; Revision = $nextMcpRev
            DesiredState = 'stopped'; Action = 'exit-all'; Source = $Source; UpdatedAtMs = $nowMs
        }
        Write-CdrAtomicJsonFile -Path (Get-CdrComponentIntentPath -RepoRoot $fullRoot -Component 'bot') -Object $botRecord
        if ($null -ne $OnAfterFirstIntentPersisted) {
            & $OnAfterFirstIntentPersisted ([pscustomobject]$botRecord)
        }
        Write-CdrAtomicJsonFile -Path (Get-CdrComponentIntentPath -RepoRoot $fullRoot -Component 'mcp') -Object $mcpRecord
        if ($null -ne $OnAfterBothIntentsPersisted) {
            & $OnAfterBothIntentsPersisted ([pscustomobject]$botRecord) ([pscustomobject]$mcpRecord)
        }

        # 3. Signal both runtimes ONLY after both durable stopped intents are on disk
        Set-CdrManualStopMarker -RepoRoot $fullRoot -Component 'bot' -Reason 'exit-all'
        Set-CdrManualStopMarker -RepoRoot $fullRoot -Component 'mcp' -Reason 'exit-all'

        $decision.State = 'committed'
        Write-CdrAtomicJsonFile -Path $exitAllPath -Object $decision

        return [pscustomobject]@{
            Bot = [pscustomobject]$botRecord
            Mcp = [pscustomobject]$mcpRecord
        }
    } finally {
        Exit-CdrIntentLock $lock
    }
}

function Read-CdrComponentLaunchJournal {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot'
    )
    $path = Get-CdrComponentLaunchJournalPath -RepoRoot $RepoRoot -Component $Component
    if (-not [IO.File]::Exists($path) -and -not [IO.Directory]::Exists($path)) { return $null }
    try {
        $item = Get-Item -LiteralPath $path -ErrorAction Stop
        if ($item.PSIsContainer) { throw 'component_launch_journal_is_directory' }
        $raw = [IO.File]::ReadAllText($path, [Text.UTF8Encoding]::UTF8)
        if ([string]::IsNullOrWhiteSpace($raw)) { throw 'component_launch_journal_empty' }
        $rec = $raw | ConvertFrom-Json -ErrorAction Stop
        if ($null -eq $rec -or
            -not $rec.PSObject.Properties['SchemaVersion'] -or [int]$rec.SchemaVersion -ne 1 -or
            -not $rec.PSObject.Properties['Component'] -or [string]$rec.Component -cne $Component -or
            -not $rec.PSObject.Properties['Phase'] -or [string]$rec.Phase -notin @('preparing', 'launching', 'launched', 'launch_outcome_unknown', 'stopped')) {
            throw 'component_launch_journal_schema_invalid'
        }
        return $rec
    } catch {
        throw "launch_outcome_unknown: $Component launch journal at $path is corrupt or unreadable ($($_.Exception.Message))"
    }
}

function Write-CdrComponentLaunchJournal {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [Parameter(Mandatory)][ValidateSet('preparing', 'launching', 'launched', 'launch_outcome_unknown', 'stopped')][string]$Phase,
        [string]$BinaryPath = '',
        [string]$Action = 'start',
        [uint64]$IntentRevision = 0,
        [int]$ProcessId = 0,
        [string]$ProcessIdentity = '',
        [string]$ArtifactHash = '',
        $BuildInfo = $null,
        [bool]$RequireStartupRecord = $false,
        [string]$Error = ''
    )
    $path = Get-CdrComponentLaunchJournalPath -RepoRoot $RepoRoot -Component $Component
    $record = [ordered]@{
        SchemaVersion        = 1
        Component            = $Component
        Phase                = $Phase
        BinaryPath           = if ($BinaryPath) { [IO.Path]::GetFullPath($BinaryPath) } else { '' }
        Action               = $Action
        IntentRevision       = [uint64]$IntentRevision
        ProcessId            = [int]$ProcessId
        ProcessIdentity      = $ProcessIdentity
        ArtifactHash         = $ArtifactHash
        BuildInfo            = $BuildInfo
        RequireStartupRecord = $RequireStartupRecord
        Error                = $Error
        UpdatedAtMs          = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    }
    Write-CdrAtomicJsonFile -Path $path -Object $record
    return [pscustomobject]$record
}

function Clear-CdrComponentLaunchJournal {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot'
    )
    $path = Get-CdrComponentLaunchJournalPath -RepoRoot $RepoRoot -Component $Component
    if ([IO.File]::Exists($path)) {
        [IO.File]::Delete($path)
    }
}

function Format-CdrWindowsCommandLine {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$ArgumentList = @()
    )
    $quoteArg = {
        param([string]$arg)
        if ($null -eq $arg) { return '""' }
        if ($arg.Length -ge 2 -and $arg.StartsWith('"') -and $arg.EndsWith('"')) { return $arg }
        if ($arg.Length -gt 0 -and $arg -notmatch '[\s"]') { return $arg }
        $escaped = $arg -replace '(\\*)"', '$1$1\"'
        $escaped = $escaped -replace '(\\+)$', '$1$1'
        return '"' + $escaped + '"'
    }
    $parts = New-Object System.Collections.Generic.List[string]
    $parts.Add((& $quoteArg $FilePath))
    foreach ($a in @($ArgumentList)) {
        $parts.Add((& $quoteArg ([string]$a)))
    }
    return ($parts -join ' ')
}

function Start-CdrDetachedProcess {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [string]$WorkingDirectory = '',
        [string]$WindowStyle = 'Hidden',
        [string]$RedirectStandardOutput = '',
        [string]$RedirectStandardError = ''
    )
    $workDir = if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) {
        Split-Path -Parent ([IO.Path]::GetFullPath($FilePath))
    } else {
        [IO.Path]::GetFullPath($WorkingDirectory)
    }

    $environment = [string[]]@([Environment]::GetEnvironmentVariables('Process').GetEnumerator() |
        ForEach-Object { [string]$_.Key + '=' + [string]$_.Value })
    $showWindow = if ($WindowStyle -eq 'Hidden') { [uint16]0 } else { [uint16]1 }
    $startup = New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{
        ShowWindow           = $showWindow
        EnvironmentVariables = $environment
    }

    if (-not [string]::IsNullOrWhiteSpace($RedirectStandardOutput) -or -not [string]::IsNullOrWhiteSpace($RedirectStandardError)) {
        $trampolineId = [guid]::NewGuid().ToString('N')
        $tempDir = [IO.Path]::GetTempPath()
        $trampolineScript = Join-Path $tempDir "cdr_trampoline_$trampolineId.ps1"
        $metaFile = Join-Path $tempDir "cdr_trampoline_$trampolineId.meta.json"
        $exitFile = Join-Path $tempDir "cdr_trampoline_$trampolineId.exit.txt"
        $ackFile = Join-Path $tempDir "cdr_trampoline_$trampolineId.ack.txt"
        $psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
        $escapedFile = $FilePath.Replace("'", "''")
        $escapedWork = $workDir.Replace("'", "''")
        $escapedWin = $WindowStyle.Replace("'", "''")
        $escapedMeta = $metaFile.Replace("'", "''")
        $escapedExit = $exitFile.Replace("'", "''")
        $escapedAck = $ackFile.Replace("'", "''")
        $argLiterals = (@($ArgumentList) | ForEach-Object {
            $s = [string]$_
            if ($s.Length -ge 2 -and $s.StartsWith('"') -and $s.EndsWith('"')) {
                "'" + $s.Replace("'", "''") + "'"
            } elseif ($s -match '[\s"]') {
                $q = '"' + ($s -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
                "'" + $q.Replace("'", "''") + "'"
            } else {
                "'" + $s.Replace("'", "''") + "'"
            }
        }) -join ', '
        $redirOutClause = if (-not [string]::IsNullOrWhiteSpace($RedirectStandardOutput)) {
            $fullOut = [IO.Path]::GetFullPath($RedirectStandardOutput).Replace("'", "''")
            " -RedirectStandardOutput '$fullOut'"
        } else { '' }
        $redirErrClause = if (-not [string]::IsNullOrWhiteSpace($RedirectStandardError)) {
            $fullErr = [IO.Path]::GetFullPath($RedirectStandardError).Replace("'", "''")
            " -RedirectStandardError '$fullErr'"
        } else { '' }
        $argClause = if ($ArgumentList -and $ArgumentList.Count -gt 0) { " -ArgumentList @($argLiterals)" } else { '' }

        $scriptBody = @"
`$ErrorActionPreference = 'Stop'
`$p = Start-Process -FilePath '$escapedFile'$argClause -WorkingDirectory '$escapedWork' -WindowStyle '$escapedWin'$redirOutClause$redirErrClause -PassThru
[void]`$p.Handle
`$ticks = `$p.StartTime.ToUniversalTime().Ticks
`$ticks -= (`$ticks % 10)
`$metaTmp = '$escapedMeta.tmp'
[IO.File]::WriteAllText(`$metaTmp, (@{ Id = [int]`$p.Id; Ticks = [long]`$ticks } | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new(`$false))
[IO.File]::Move(`$metaTmp, '$escapedMeta')
`$deadline = [DateTime]::UtcNow.AddSeconds(3)
while (-not [IO.File]::Exists('$escapedAck') -and [DateTime]::UtcNow -lt `$deadline) {
    if (`$p.WaitForExit(40)) {
        [IO.File]::WriteAllText('$escapedExit', [string]`$p.ExitCode, [Text.UTF8Encoding]::new(`$false))
        break
    }
}
if (`$p.HasExited -and -not [IO.File]::Exists('$escapedExit')) {
    [IO.File]::WriteAllText('$escapedExit', [string]`$p.ExitCode, [Text.UTF8Encoding]::new(`$false))
}
"@
        try {
            [IO.File]::WriteAllText($trampolineScript, $scriptBody, [Text.UTF8Encoding]::new($false))
            $trampolineCmd = Format-CdrWindowsCommandLine -FilePath $psExe -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', $trampolineScript)
            $result = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
                CommandLine               = $trampolineCmd
                CurrentDirectory          = $workDir
                ProcessStartupInformation = $startup
            }
            if ($null -eq $result -or $result.ReturnValue -ne 0 -or -not $result.ProcessId) {
                $rv = if ($null -ne $result) { $result.ReturnValue } else { 'null' }
                throw "Win32_Process.Create failed with return value $rv for $FilePath"
            }
            $metaDeadline = [DateTime]::UtcNow.AddSeconds(8)
            while (-not [IO.File]::Exists($metaFile) -and [DateTime]::UtcNow -lt $metaDeadline) {
                Start-Sleep -Milliseconds 20
            }
            if (-not [IO.File]::Exists($metaFile)) {
                throw "Detached redirected launch did not produce process metadata for $FilePath"
            }
            $meta = [IO.File]::ReadAllText($metaFile, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json
            $metaPid = [int]$meta.Id
            $metaTicks = [long]$meta.Ticks
            $metaTicks -= ($metaTicks % 10)
            $expectedMetaIdentity = "$metaPid|$metaTicks"
            $proc = $null
            $verifiedLiveHandle = $false
            try {
                $proc = Get-Process -Id $metaPid -ErrorAction Stop
                [void]$proc.Handle
                $actualMetaIdentity = Get-CdrProcessIdentity -Process $proc
                if ($actualMetaIdentity -ceq $expectedMetaIdentity) {
                    $verifiedLiveHandle = $true
                } else {
                    $proc.Dispose()
                    $proc = $null
                }
            } catch {
                if ($null -ne $proc) {
                    try { $proc.Dispose() } catch { }
                    $proc = $null
                }
            }
            if (-not $verifiedLiveHandle -or ($proc.HasExited -and $null -eq $proc.ExitCode)) {
                $exDeadline = [DateTime]::UtcNow.AddSeconds(2)
                while (-not [IO.File]::Exists($exitFile) -and [DateTime]::UtcNow -lt $exDeadline) {
                    Start-Sleep -Milliseconds 20
                }
                [IO.File]::WriteAllText($ackFile, '1', [Text.UTF8Encoding]::new($false))
                if (-not [IO.File]::Exists($exitFile)) {
                    if ($null -ne $proc) { try { $proc.Dispose() } catch { } }
                    throw "Detached redirected launch could not verify live handle or exit status for $expectedMetaIdentity ($FilePath)"
                }
                $ec = [int]([IO.File]::ReadAllText($exitFile).Trim())
                if ($null -ne $proc) { try { $proc.Dispose() } catch { } }
                $proc = [pscustomobject]@{
                    Id        = $metaPid
                    Path      = [IO.Path]::GetFullPath($FilePath)
                    StartTime = [DateTime]::new($metaTicks, [DateTimeKind]::Utc)
                    HasExited = $true
                    ExitCode  = $ec
                }
                $proc | Add-Member ScriptMethod Refresh {}
                $proc | Add-Member ScriptMethod WaitForExit { param($ms) return $true }
            } else {
                [IO.File]::WriteAllText($ackFile, '1', [Text.UTF8Encoding]::new($false))
            }
            return $proc
        } finally {
            foreach ($tmpPath in @($trampolineScript, $metaFile, "$metaFile.tmp", $exitFile, $ackFile)) {
                Remove-Item -LiteralPath $tmpPath -Force -ErrorAction SilentlyContinue
            }
        }
    }

    # Real detached execution via Win32_Process.Create (hosted by WmiPrvSE.exe),
    # completely escaping CapturedWindowProcess JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE and caller tree.
    $commandLine = Format-CdrWindowsCommandLine -FilePath $FilePath -ArgumentList $ArgumentList
    $result = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
        CommandLine               = $commandLine
        CurrentDirectory          = $workDir
        ProcessStartupInformation = $startup
    }
    if ($null -eq $result -or $result.ReturnValue -ne 0 -or -not $result.ProcessId) {
        $rv = if ($null -ne $result) { $result.ReturnValue } else { 'null' }
        throw "Win32_Process.Create failed with return value $rv for $FilePath"
    }
    $spawnedProc = Get-Process -Id ([int]$result.ProcessId) -ErrorAction Stop
    try { [void]$spawnedProc.Handle } catch { }
    return $spawnedProc
}

function Invoke-CdrJournaledProcessLaunch {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [string]$WorkingDirectory = '',
        [string]$TargetBinaryPath = '',
        [string]$Action = 'start',
        [string]$WindowStyle = 'Hidden',
        [string]$RedirectStandardOutput = '',
        [string]$RedirectStandardError = '',
        [uint64]$ExpectedIntentRevision = 0,
        [string]$ExpectedBinaryHash = '',
        [string]$ExpectedSourceCommit = '',
        [switch]$AllowClearManualStop
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $resolvedTarget = if ([string]::IsNullOrWhiteSpace($TargetBinaryPath)) {
        [IO.Path]::GetFullPath($FilePath)
    } else {
        [IO.Path]::GetFullPath($TargetBinaryPath)
    }
    $workDir = if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) {
        $fullRoot
    } else {
        [IO.Path]::GetFullPath($WorkingDirectory)
    }

    $artifactHash = $ExpectedBinaryHash
    $buildInfo = $null
    $requireStartupRecord = $false
    $artifactPin = $null
    if ([IO.File]::Exists($resolvedTarget)) {
        $targetItem = Get-Item -LiteralPath $resolvedTarget -ErrorAction Stop
        $isCdrBinary = ($targetItem.Name -in @('cdr-runtime.exe', 'cdr-remote-agent.exe'))
        if (($ExpectedBinaryHash -or $ExpectedSourceCommit -or $isCdrBinary) -and (Get-Command Get-CdrRuntimeArtifact -ErrorAction SilentlyContinue)) {
            $art = Get-CdrRuntimeArtifact -Path $resolvedTarget -ExpectedBinaryHash $ExpectedBinaryHash -ExpectedSourceCommit $ExpectedSourceCommit
            $artifactHash = $art.ArtifactHash
            $buildInfo = $art.BuildInfo
            $requireStartupRecord = [bool]$art.RequireStartupRecord
            $artifactPin = Get-CdrRuntimeArtifactPin -Path $resolvedTarget -ExpectedHash $artifactHash
        } else {
            $artifactHash = (Get-FileHash -LiteralPath $resolvedTarget -Algorithm SHA256).Hash.ToUpperInvariant()
            if ((Get-Command Get-CdrRuntimeArtifactPin -ErrorAction SilentlyContinue) -and $artifactHash -match '^[a-fA-F0-9]{64}$') {
                $artifactPin = Get-CdrRuntimeArtifactPin -Path $resolvedTarget -ExpectedHash $artifactHash
            }
        }
    }

    $lockToken = Enter-CdrComponentShortLock -RepoRoot $fullRoot -Component $Component
    try {
        $intent = $null
        $proc = $null
        $identity = ''
        $journal = $null
        $intentLock = Enter-CdrIntentLock -RepoRoot $fullRoot
        try {
            if ($Component -eq 'bot') {
                Assert-CdrNoUncertainLegacyBotLaunchJournal -RepoRoot $fullRoot
            }

            $existingJournal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component
            if ($null -ne $existingJournal) {
                if ($existingJournal.Phase -eq 'launching') {
                    Write-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component `
                        -Phase 'launch_outcome_unknown' -BinaryPath $resolvedTarget -Action $existingJournal.Action `
                        -IntentRevision ([uint64]$existingJournal.IntentRevision) `
                        -Error 'Previous launch crashed in launching phase before recording child identity' | Out-Null
                    throw 'launch_outcome_unknown: previous launch crashed before recording child identity; duplicate launch refused'
                }
                if ($existingJournal.Phase -eq 'launch_outcome_unknown') {
                    throw 'launch_outcome_unknown: uncertain spawn hold active; duplicate launch refused'
                }
                if ($existingJournal.Phase -eq 'launched' -and $existingJournal.ProcessId -gt 0 -and $existingJournal.ProcessIdentity) {
                    $obs = Get-CdrProcessObservation -ProcessId ([int]$existingJournal.ProcessId) -ExpectedIdentity ([string]$existingJournal.ProcessIdentity) -ExpectedBinaryPath $resolvedTarget
                    if ($obs.State -eq 'running') {
                        throw "cdr_already_running: $Component process $($existingJournal.ProcessIdentity) is already alive; duplicate launch refused"
                    }
                    if ($obs.State -eq 'unknown') {
                        throw "launch_outcome_unknown: existing $Component process $($existingJournal.ProcessIdentity) observation is unknown ($($obs.Reason)); duplicate launch refused"
                    }
                }
            }

            $liveObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component $Component -BinaryPath $resolvedTarget
            if ($liveObs.State -eq 'running') {
                throw "cdr_already_running: $Component process $($liveObs.ProcessIdentity) is already alive; duplicate launch refused"
            }
            if ($liveObs.State -eq 'unknown') {
                throw "launch_outcome_unknown: $Component live observation is unknown ($($liveObs.Reason)); duplicate launch refused"
            }

            if ($AllowClearManualStop -and $ExpectedIntentRevision -eq 0) {
                Set-CdrComponentIntent -RepoRoot $fullRoot -Component $Component -DesiredState 'running' -Action $Action -Source $Action | Out-Null
            }

            $intent = Get-CdrComponentIntent -RepoRoot $fullRoot -Component $Component
            if ($ExpectedIntentRevision -gt 0 -and $intent.Revision -ne $ExpectedIntentRevision) {
                throw "cdr_stale_intent_revision: $Component intent revision changed (expected=$ExpectedIntentRevision, actual=$($intent.Revision)); launch refused"
            }
            if ($intent.DesiredState -eq 'stopped' -or (Test-CdrManualStopRequested -RepoRoot $fullRoot -Component $Component)) {
                throw "cdr_manual_stop_active: $Component desired state is stopped (revision=$($intent.Revision)); launch refused"
            }

            Write-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component `
                -Phase 'launching' -BinaryPath $resolvedTarget -Action $Action `
                -IntentRevision ([uint64]$intent.Revision) -ArtifactHash $artifactHash `
                -BuildInfo $buildInfo -RequireStartupRecord $requireStartupRecord | Out-Null

            try {
                if (-not [string]::IsNullOrWhiteSpace($RedirectStandardOutput) -or -not [string]::IsNullOrWhiteSpace($RedirectStandardError)) {
                    $proc = Start-CdrDetachedProcess -FilePath $FilePath -ArgumentList $ArgumentList -WorkingDirectory $workDir -WindowStyle $WindowStyle `
                        -RedirectStandardOutput $RedirectStandardOutput -RedirectStandardError $RedirectStandardError
                } else {
                    $proc = Start-CdrDetachedProcess -FilePath $FilePath -ArgumentList $ArgumentList -WorkingDirectory $workDir -WindowStyle $WindowStyle
                }
                if ($null -eq $proc) {
                    throw 'Detached spawn returned null process object'
                }
                $identity = Get-CdrProcessIdentity -Process $proc
                if ([string]::IsNullOrWhiteSpace($identity)) {
                    throw "Detached spawn PID $($proc.Id) did not yield a verifiable process identity"
                }
            } catch {
                $errMsg = $_.Exception.Message
                Write-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component `
                    -Phase 'launch_outcome_unknown' -BinaryPath $resolvedTarget -Action $Action `
                    -IntentRevision ([uint64]$intent.Revision) -ArtifactHash $artifactHash `
                    -BuildInfo $buildInfo -RequireStartupRecord $requireStartupRecord -Error $errMsg | Out-Null
                throw "launch_outcome_unknown: $errMsg"
            }

            $journal = Write-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component `
                -Phase 'launched' -BinaryPath $resolvedTarget -Action $Action `
                -IntentRevision ([uint64]$intent.Revision) -ProcessId ([int]$proc.Id) -ProcessIdentity $identity `
                -ArtifactHash $artifactHash -BuildInfo $buildInfo -RequireStartupRecord $requireStartupRecord
        } finally {
            Exit-CdrIntentLock $intentLock
        }

        return [pscustomobject]@{
            Process         = $proc
            ProcessId       = [int]$proc.Id
            ProcessIdentity = $identity
            IntentRevision  = [uint64]$intent.Revision
            Superseded      = $false
            Journal         = $journal
        }
    } finally {
        Exit-CdrComponentShortLock $lockToken
        if ($null -ne $artifactPin) { $artifactPin.Dispose() }
    }
}

function Resolve-CdrComponentBinaryCandidates {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [string]$BinaryPath = ''
    )
    $list = New-Object System.Collections.Generic.List[string]
    if (-not [string]::IsNullOrWhiteSpace($BinaryPath)) {
        $fullBin = [IO.Path]::GetFullPath($BinaryPath)
        if ([IO.File]::Exists($fullBin)) {
            $list.Add($fullBin)
        }
        return ,$list.ToArray()
    }
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $exeName = if ($Component -eq 'mcp') { 'cdr-remote-agent.exe' } else { 'cdr-runtime.exe' }
    foreach ($rel in @("bin\win-x64\$exeName", "bin\$exeName", "target\release\$exeName")) {
        $cand = Join-Path $fullRoot $rel
        if ([IO.File]::Exists($cand)) {
            $list.Add([IO.Path]::GetFullPath($cand))
        }
    }
    return ,$list.ToArray()
}

function Read-CdrMcpInstanceLockRecord {
    param([Parameter(Mandatory)][string]$LockPath)
    if (-not [IO.File]::Exists($LockPath)) { return $null }
    $item = Get-Item -LiteralPath $LockPath -ErrorAction Stop
    if ($item.PSIsContainer) { throw 'mcp_instance_lock_is_directory' }
    $raw = [IO.File]::ReadAllText($LockPath, [Text.UTF8Encoding]::UTF8).Trim()
    if ([string]::IsNullOrWhiteSpace($raw)) {
        throw 'mcp_instance_lock_empty'
    }
    if ($raw -match '^(\d+)\|(\d+)$') {
        return [pscustomobject]@{
            ProcessId       = [int]$Matches[1]
            ProcessIdentity = "$($Matches[1])|$($Matches[2])"
            ExecutablePath  = ''
        }
    }
    if ($raw -match '(?m)^identity=((\d+)\|\d+)\r?$') {
        return [pscustomobject]@{
            ProcessId       = [int]$Matches[2]
            ProcessIdentity = [string]$Matches[1]
            ExecutablePath  = ''
        }
    }
    if ($raw.StartsWith('{')) {
        $json = $raw | ConvertFrom-Json -ErrorAction Stop
        $pidVal = [int]$json.pid
        $idVal = [string]$json.process_identity
        if ($pidVal -le 0 -or $idVal -notmatch '^\d+\|\d+$') {
            throw 'mcp_instance_lock_json_invalid'
        }
        return [pscustomobject]@{
            ProcessId       = $pidVal
            ProcessIdentity = $idVal
            ExecutablePath  = if ($json.PSObject.Properties['executable_path']) { [string]$json.executable_path } else { '' }
        }
    }
    throw 'mcp_instance_lock_malformed'
}

function Get-CdrComponentObservation {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [string]$BinaryPath = ''
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $candidates = Resolve-CdrComponentBinaryCandidates -RepoRoot $fullRoot -Component $Component -BinaryPath $BinaryPath

    # 1. Check component launch journal first (controller-owned authority)
    $journalPath = Get-CdrComponentLaunchJournalPath -RepoRoot $fullRoot -Component $Component
    if ([IO.File]::Exists($journalPath) -or [IO.Directory]::Exists($journalPath)) {
        $journal = $null
        try {
            $journal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component
        } catch {
            return [pscustomobject]@{
                Component = $Component; State = 'unknown'; ProcessId = 0; ProcessIdentity = ''; BinaryPath = ''; Process = $null
                HasControlAuthority = $false; Reason = "launch_journal_corrupt:$($_.Exception.Message)"
            }
        }
        if ($null -ne $journal -and $journal.ProcessId -gt 0 -and -not [string]::IsNullOrWhiteSpace($journal.ProcessIdentity)) {
            $expectedBin = if (-not [string]::IsNullOrWhiteSpace($BinaryPath)) {
                [IO.Path]::GetFullPath($BinaryPath)
            } elseif (-not [string]::IsNullOrWhiteSpace($journal.BinaryPath)) {
                [IO.Path]::GetFullPath($journal.BinaryPath)
            } else {
                ''
            }
            $obs = Get-CdrProcessObservation -ProcessId ([int]$journal.ProcessId) -ExpectedIdentity ([string]$journal.ProcessIdentity) -ExpectedBinaryPath $expectedBin
            if ($obs.State -eq 'running') {
                return [pscustomobject]@{
                    Component           = $Component
                    State               = 'running'
                    ProcessId           = [int]$obs.ProcessId
                    ProcessIdentity     = [string]$obs.ProcessIdentity
                    BinaryPath          = [string]$obs.BinaryPath
                    Process             = $obs.Process
                    HasControlAuthority = $true
                    Reason              = 'matched_journal'
                }
            }
            if ($obs.State -eq 'unknown') {
                return [pscustomobject]@{
                    Component           = $Component
                    State               = 'unknown'
                    ProcessId           = [int]$journal.ProcessId
                    ProcessIdentity     = [string]$journal.ProcessIdentity
                    BinaryPath          = $expectedBin
                    Process             = $null
                    HasControlAuthority = $false
                    Reason              = "journal_process_unknown:$($obs.Reason)"
                }
            }
        }
    }

    # 2. Check component lock / status files
    if ($Component -eq 'bot') {
        $lockPath = Join-Path $fullRoot '.codex_discord_rust.runtime.lock'
        if ([IO.File]::Exists($lockPath) -or [IO.Directory]::Exists($lockPath)) {
            try {
                $item = Get-Item -LiteralPath $lockPath -ErrorAction Stop
                if ($item.PSIsContainer) { throw 'bot_lock_is_directory' }
                $text = [IO.File]::ReadAllText($lockPath, [Text.UTF8Encoding]::UTF8)
                $pidMatches = [regex]::Matches($text, '(?m)^pid=([^\r\n]*)\r?$')
                if ($pidMatches.Count -ne 1) {
                    throw 'bot_lock_pid_missing_or_duplicate'
                }
                $pidVal = 0
                if (-not [int]::TryParse($pidMatches[0].Groups[1].Value.Trim(), [ref]$pidVal) -or $pidVal -le 0) {
                    throw 'bot_lock_pid_invalid'
                }
                $startupLines = @($text -split "`r?`n" | Where-Object { $_.StartsWith('startup=') })
                $idLines = @($text -split "`r?`n" | Where-Object { $_ -match '^identity=([^\r\n]*)\r?$' })
                if ($startupLines.Count -gt 1 -or $idLines.Count -gt 1) {
                    throw 'bot_lock_identity_ambiguous'
                }
                $parsedStartup = $null
                if ($startupLines.Count -eq 1) {
                    try {
                        $parsedStartup = $startupLines[0].Substring(8) | ConvertFrom-Json -ErrorAction Stop
                    } catch {
                        throw 'bot_lock_startup_json_corrupt'
                    }
                    if ($null -eq $parsedStartup -or
                        -not $parsedStartup.PSObject.Properties['schema_version'] -or [int]$parsedStartup.schema_version -ne 1 -or
                        -not $parsedStartup.PSObject.Properties['process_identity'] -or [string]$parsedStartup.process_identity -notmatch '^\d+\|\d+$' -or
                        -not $parsedStartup.PSObject.Properties['executable_path'] -or [string]::IsNullOrWhiteSpace($parsedStartup.executable_path) -or
                        -not $parsedStartup.PSObject.Properties['artifact_sha256'] -or [string]$parsedStartup.artifact_sha256 -cnotmatch '^[a-f0-9]{64}$') {
                        throw 'bot_lock_startup_schema_invalid'
                    }
                }
                $parsedLockId = ''
                if ($idLines.Count -eq 1) {
                    $parsedLockId = ($idLines[0] -replace '^identity=', '').Trim()
                    if ($parsedLockId -notmatch '^\d+\|\d+$') {
                        throw 'bot_lock_identity_invalid'
                    }
                }
                $proc = $null
                try {
                    $proc = Get-Process -Id $pidVal -ErrorAction Stop
                } catch {
                    if ($_.FullyQualifiedErrorId -like 'NoProcessFoundForGivenId*' -or
                        $_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound -or
                        $_.Exception.Message -match 'Cannot find a process with the process identifier') {
                        return [pscustomobject]@{ Component = 'bot'; State = 'stopped'; ProcessId = 0; ProcessIdentity = ''; BinaryPath = ''; Process = $null; HasControlAuthority = $false; Reason = 'lock_pid_not_found' }
                    }
                    throw
                }
                if ($null -eq $proc -or ($proc.PSObject.Properties['HasExited'] -and $proc.HasExited)) {
                    return [pscustomobject]@{ Component = 'bot'; State = 'stopped'; ProcessId = 0; ProcessIdentity = ''; BinaryPath = ''; Process = $null; HasControlAuthority = $false; Reason = 'lock_pid_exited' }
                }
                $identity = Get-CdrProcessIdentity -Process $proc
                if ([string]::IsNullOrWhiteSpace($identity)) {
                    throw 'bot_lock_process_identity_unavailable'
                }
                $checkBins = if ($candidates.Count -gt 0) { $candidates } else { @((Join-Path $fullRoot 'target\release\cdr-runtime.exe'), (Join-Path $fullRoot 'bin\win-x64\cdr-runtime.exe')) }
                foreach ($cand in $checkBins) {
                    $obs = Get-CdrProcessObservation -ProcessId $pidVal -ExpectedIdentity $identity -ExpectedBinaryPath $cand -Process $proc
                    if ($obs.State -eq 'running') {
                        $hasLockAuthority = $false
                        $lockReason = 'matched_legacy_lock_pid_only'
                        if ($startupLines.Count -eq 1) {
                            $startupRec = if (Get-Command Read-CdrRuntimeStartup -ErrorAction SilentlyContinue) {
                                Read-CdrRuntimeStartup -Root $fullRoot -BinaryPath $cand -ExpectedIdentity $identity
                            } else {
                                if ([string]$parsedStartup.process_identity -ceq $identity -and
                                    [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath([string]$parsedStartup.executable_path), [IO.Path]::GetFullPath($cand))) {
                                    $parsedStartup
                                } else {
                                    $null
                                }
                            }
                            if ($null -ne $startupRec) {
                                if ([string]::IsNullOrWhiteSpace($parsedLockId) -or $parsedLockId -ceq $identity) {
                                    $hasLockAuthority = $true
                                    $lockReason = 'matched_lock'
                                } else {
                                    $hasLockAuthority = $false
                                    $lockReason = 'lock_identity_mismatch'
                                }
                            } else {
                                $hasLockAuthority = $false
                                $lockReason = 'lock_startup_identity_mismatch'
                            }
                        } elseif (-not [string]::IsNullOrWhiteSpace($parsedLockId)) {
                            if ($parsedLockId -ceq $identity) {
                                $hasLockAuthority = $true
                                $lockReason = 'matched_lock'
                            } else {
                                $hasLockAuthority = $false
                                $lockReason = 'lock_identity_mismatch'
                            }
                        }
                        return [pscustomobject]@{
                            Component           = 'bot'
                            State               = 'running'
                            ProcessId           = $pidVal
                            ProcessIdentity     = $identity
                            BinaryPath          = $cand
                            Process             = $obs.Process
                            HasControlAuthority = $hasLockAuthority
                            Reason              = $lockReason
                        }
                    }
                    if ($obs.State -eq 'unknown') {
                        return [pscustomobject]@{
                            Component           = 'bot'
                            State               = 'unknown'
                            ProcessId           = $pidVal
                            ProcessIdentity     = $identity
                            BinaryPath          = $cand
                            Process             = $null
                            HasControlAuthority = $false
                            Reason              = "lock_process_unknown:$($obs.Reason)"
                        }
                    }
                }
            } catch {
                return [pscustomobject]@{
                    Component = 'bot'; State = 'unknown'; ProcessId = 0; ProcessIdentity = ''; BinaryPath = ''; Process = $null
                    HasControlAuthority = $false; Reason = "bot_lock_observation_failed:$($_.Exception.Message)"
                }
            }
        }
    } else {
        $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
        if ([IO.File]::Exists($mcpPaths.InstanceLockPath) -or [IO.Directory]::Exists($mcpPaths.InstanceLockPath)) {
            try {
                $lockRec = Read-CdrMcpInstanceLockRecord -LockPath $mcpPaths.InstanceLockPath
                if ($null -ne $lockRec) {
                    $checkBins = if (-not [string]::IsNullOrWhiteSpace($BinaryPath)) {
                        @([IO.Path]::GetFullPath($BinaryPath))
                    } elseif (-not [string]::IsNullOrWhiteSpace($lockRec.ExecutablePath)) {
                        @([IO.Path]::GetFullPath($lockRec.ExecutablePath))
                    } elseif ($candidates.Count -gt 0) {
                        $candidates
                    } else {
                        @('')
                    }
                    foreach ($cand in $checkBins) {
                        $obs = Get-CdrProcessObservation -ProcessId $lockRec.ProcessId -ExpectedIdentity $lockRec.ProcessIdentity -ExpectedBinaryPath $cand
                        if ($obs.State -eq 'running') {
                            return [pscustomobject]@{
                                Component           = 'mcp'
                                State               = 'running'
                                ProcessId           = $lockRec.ProcessId
                                ProcessIdentity     = $lockRec.ProcessIdentity
                                BinaryPath          = $obs.BinaryPath
                                Process             = $obs.Process
                                HasControlAuthority = $true
                                Reason              = 'matched_instance_lock'
                            }
                        }
                        if ($obs.State -eq 'unknown') {
                            return [pscustomobject]@{
                                Component           = 'mcp'
                                State               = 'unknown'
                                ProcessId           = $lockRec.ProcessId
                                ProcessIdentity     = $lockRec.ProcessIdentity
                                BinaryPath          = $cand
                                Process             = $null
                                HasControlAuthority = $false
                                Reason              = "instance_lock_process_unknown:$($obs.Reason)"
                            }
                        }
                    }
                }
            } catch {
                return [pscustomobject]@{
                    Component = 'mcp'; State = 'unknown'; ProcessId = 0; ProcessIdentity = ''; BinaryPath = ''; Process = $null
                    HasControlAuthority = $false; Reason = "mcp_instance_lock_corrupt:$($_.Exception.Message)"
                }
            }
        }

        $statusCandidates = @($mcpPaths.StatusPath, (Join-Path $fullRoot 'bot_data\remote_mcp.status.json')) | Select-Object -Unique
        foreach ($stPath in $statusCandidates) {
            if ([IO.File]::Exists($stPath) -or [IO.Directory]::Exists($stPath)) {
                try {
                    $item = Get-Item -LiteralPath $stPath -ErrorAction Stop
                    if ($item.PSIsContainer) { throw 'mcp_status_is_directory' }
                    $rec = [IO.File]::ReadAllText($stPath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
                    $pidVal = [int]$rec.pid
                    $idVal = [string]$rec.process_identity
                    $exeVal = if (-not [string]::IsNullOrWhiteSpace($BinaryPath)) {
                        [IO.Path]::GetFullPath($BinaryPath)
                    } elseif (-not [string]::IsNullOrWhiteSpace($rec.executable_path)) {
                        [IO.Path]::GetFullPath([string]$rec.executable_path)
                    } else {
                        ''
                    }
                    $obs = Get-CdrProcessObservation -ProcessId $pidVal -ExpectedIdentity $idVal -ExpectedBinaryPath $exeVal
                    if ($obs.State -eq 'running') {
                        # Status observation alone never grants force-termination authority!
                        return [pscustomobject]@{
                            Component           = 'mcp'
                            State               = 'running'
                            ProcessId           = $pidVal
                            ProcessIdentity     = $idVal
                            BinaryPath          = $obs.BinaryPath
                            Process             = $obs.Process
                            HasControlAuthority = $false
                            Reason              = 'matched_status'
                        }
                    }
                    if ($obs.State -eq 'unknown') {
                        return [pscustomobject]@{
                            Component           = 'mcp'
                            State               = 'unknown'
                            ProcessId           = $pidVal
                            ProcessIdentity     = $idVal
                            BinaryPath          = $exeVal
                            Process             = $null
                            HasControlAuthority = $false
                            Reason              = "status_process_unknown:$($obs.Reason)"
                        }
                    }
                } catch {
                    return [pscustomobject]@{
                        Component = 'mcp'; State = 'unknown'; ProcessId = 0; ProcessIdentity = ''; BinaryPath = ''; Process = $null
                        HasControlAuthority = $false; Reason = "mcp_status_corrupt:$($_.Exception.Message)"
                    }
                }
            }
        }
    }

    return [pscustomobject]@{
        Component           = $Component
        State               = 'stopped'
        ProcessId           = 0
        ProcessIdentity     = ''
        BinaryPath          = ''
        Process             = $null
        HasControlAuthority = $false
        Reason              = 'not_running'
    }
}

function Get-CdrComponentLiveProcess {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [string]$BinaryPath = ''
    )
    $obs = Get-CdrComponentObservation -RepoRoot $RepoRoot -Component $Component -BinaryPath $BinaryPath
    if ($obs.State -eq 'running') {
        return $obs
    }
    return $null
}

function Stop-CdrComponentRuntime {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [ValidateSet('bot', 'mcp')][string]$Component = 'bot',
        [string]$BinaryPath = '',
        [int]$GraceTimeoutMs = 45000,
        [string]$Source = 'manual-stop',
        [switch]$PreserveIntent,
        [uint64]$ExpectedIntentRevision = 0,
        [string]$ExpectedDesiredState = '',
        [switch]$AllowForceKill,
        [scriptblock]$KillOverride = $null
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $stopOperationRevision = [uint64]0
    $stopOperationDesiredState = 'stopped'
    $obs = $null
    $earlyReturn = $null
    $initLock = Enter-CdrIntentLock -RepoRoot $fullRoot
    try {
        if (-not $PreserveIntent) {
            $stopIntentRec = Set-CdrComponentIntent -RepoRoot $fullRoot -Component $Component -DesiredState 'stopped' -Action 'stop' -Source $Source
            $stopOperationRevision = [uint64]$stopIntentRec.Revision
            $stopOperationDesiredState = 'stopped'
        } else {
            $curIntent = Get-CdrComponentIntent -RepoRoot $fullRoot -Component $Component
            $effectiveExpectedRev = $ExpectedIntentRevision
            $effectiveExpectedState = $ExpectedDesiredState
            if ($effectiveExpectedRev -eq 0 -and ($Source -like '*exit-all*' -or [string]$curIntent.Action -eq 'exit-all')) {
                $exitAllPath = Get-CdrExitAllDecisionPath -RepoRoot $fullRoot
                if ([IO.File]::Exists($exitAllPath)) {
                    try {
                        $exitDecision = [IO.File]::ReadAllText($exitAllPath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
                        if ($null -ne $exitDecision) {
                            $revProp = if ($Component -eq 'bot') { 'BotRevision' } else { 'McpRevision' }
                            if ($exitDecision.PSObject.Properties[$revProp] -and [uint64]$exitDecision.$revProp -gt 0) {
                                $effectiveExpectedRev = [uint64]$exitDecision.$revProp
                            }
                        }
                    } catch { }
                }
                if ([string]::IsNullOrWhiteSpace($effectiveExpectedState)) {
                    $effectiveExpectedState = 'stopped'
                }
            }
            $intentMatches = $true
            if ($effectiveExpectedRev -gt 0 -and [uint64]$curIntent.Revision -ne $effectiveExpectedRev) {
                $intentMatches = $false
            }
            if (-not [string]::IsNullOrWhiteSpace($effectiveExpectedState) -and [string]$curIntent.DesiredState -cne $effectiveExpectedState) {
                $intentMatches = $false
            }
            if ($Source -like '*exit-all*' -and ([string]$curIntent.DesiredState -cne 'stopped' -or [string]$curIntent.Action -cne 'exit-all')) {
                $intentMatches = $false
            }
            if (-not $intentMatches) {
                $obsNow = Get-CdrComponentObservation -RepoRoot $fullRoot -Component $Component -BinaryPath $BinaryPath
                $earlyReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = if ($obsNow.State -eq 'running') { 'still_running' } else { [string]$obsNow.State }
                    Stopped         = $false
                    WasRunning      = [bool]($obsNow.State -eq 'running')
                    ProcessId       = [int]$obsNow.ProcessId
                    ProcessIdentity = [string]$obsNow.ProcessIdentity
                    Reason          = 'superseded_by_newer_intent'
                }
            } else {
                $stopOperationRevision = [uint64]$curIntent.Revision
                $stopOperationDesiredState = [string]$curIntent.DesiredState
                Set-CdrManualStopMarker -RepoRoot $fullRoot -Component $Component -Reason $Source
            }
        }

        if ($null -eq $earlyReturn -and $Component -eq 'bot') {
            try {
                Assert-CdrNoUncertainLegacyBotLaunchJournal -RepoRoot $fullRoot
            } catch {
                $earlyReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = 'unknown'
                    Stopped         = $false
                    WasRunning      = $false
                    ProcessId       = 0
                    ProcessIdentity = ''
                    Reason          = $_.Exception.Message
                }
            }
        }

        $initJournal = $null
        if ($null -eq $earlyReturn) {
            try {
                $initJournal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component
            } catch {
                $earlyReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = 'unknown'
                    Stopped         = $false
                    WasRunning      = $false
                    ProcessId       = 0
                    ProcessIdentity = ''
                    Reason          = $_.Exception.Message
                }
            }
        }

        if ($null -eq $earlyReturn -and $null -ne $initJournal) {
            if ($initJournal.Phase -in @('launching', 'launch_outcome_unknown')) {
                $earlyReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = 'unknown'
                    Stopped         = $false
                    WasRunning      = $false
                    ProcessId       = [int]$initJournal.ProcessId
                    ProcessIdentity = [string]$initJournal.ProcessIdentity
                    Reason          = "launch_outcome_unknown:$($initJournal.Phase)"
                }
            } elseif ([uint64]$initJournal.IntentRevision -gt $stopOperationRevision) {
                $earlyReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = 'still_running'
                    Stopped         = $false
                    WasRunning      = $true
                    ProcessId       = [int]$initJournal.ProcessId
                    ProcessIdentity = [string]$initJournal.ProcessIdentity
                    Reason          = 'superseded_by_newer_intent'
                }
            }
        }

        if ($null -eq $earlyReturn) {
            $obs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component $Component -BinaryPath $BinaryPath
            if ($obs.State -eq 'unknown') {
                $earlyReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = 'unknown'
                    Stopped         = $false
                    WasRunning      = $false
                    ProcessId       = [int]$obs.ProcessId
                    ProcessIdentity = [string]$obs.ProcessIdentity
                    Reason          = [string]$obs.Reason
                }
            } elseif ($obs.State -eq 'stopped') {
                $curIntent = Get-CdrComponentIntent -RepoRoot $fullRoot -Component $Component
                $intentStillCurrent = ([uint64]$curIntent.Revision -eq $stopOperationRevision -and [string]$curIntent.DesiredState -eq $stopOperationDesiredState)
                if ($intentStillCurrent -and $null -ne $initJournal -and $initJournal.Phase -eq 'launched' -and
                    -not [string]::IsNullOrWhiteSpace($initJournal.ProcessIdentity) -and [uint64]$initJournal.IntentRevision -le $stopOperationRevision) {
                    $postCheck = Get-CdrProcessObservation -ProcessId ([int]$initJournal.ProcessId) -ExpectedIdentity ([string]$initJournal.ProcessIdentity) -ExpectedBinaryPath ([string]$initJournal.BinaryPath)
                    if ($postCheck.State -eq 'stopped') {
                        Write-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component `
                            -Phase 'stopped' -BinaryPath $initJournal.BinaryPath -Action 'stop' `
                            -IntentRevision $stopOperationRevision `
                            -ProcessId ([int]$initJournal.ProcessId) -ProcessIdentity ([string]$initJournal.ProcessIdentity) | Out-Null
                    }
                }
                $earlyReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = 'stopped'
                    Stopped         = $true
                    WasRunning      = $false
                    ProcessId       = 0
                    ProcessIdentity = ''
                    Reason          = 'already_stopped'
                }
            }
        }
    } finally {
        Exit-CdrIntentLock $initLock
    }
    if ($null -ne $earlyReturn) {
        return $earlyReturn
    }

    $live = $obs
    $hasControlAuthority = [bool]($live.PSObject.Properties['HasControlAuthority'] -and $live.HasControlAuthority -and $live.Reason -ne 'matched_status')
    $deadline = [DateTime]::UtcNow.AddMilliseconds($GraceTimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        $check = Get-CdrProcessObservation -ProcessId $live.ProcessId -ExpectedIdentity $live.ProcessIdentity -ExpectedBinaryPath $live.BinaryPath -Process $live.Process
        if ($check.State -eq 'stopped') {
            break
        }
        if ([Environment]::UserInteractive -and ('System.Windows.Forms.Application' -as [type])) {
            try { [System.Windows.Forms.Application]::DoEvents() } catch { }
        }
        Start-Sleep -Milliseconds 50
    }

    $postWait = Get-CdrProcessObservation -ProcessId $live.ProcessId -ExpectedIdentity $live.ProcessIdentity -ExpectedBinaryPath $live.BinaryPath -Process $live.Process
    if ($postWait.State -ne 'stopped' -and ($AllowForceKill -or $null -ne $KillOverride)) {
        if (-not $hasControlAuthority) {
            return [pscustomobject]@{
                Component       = $Component
                State           = 'unknown'
                Stopped         = $false
                WasRunning      = $true
                ProcessId       = [int]$live.ProcessId
                ProcessIdentity = [string]$live.ProcessIdentity
                Reason          = 'status_only_no_kill_authority'
            }
        }
        $forceReturn = $null
        $forceCommitted = $false
        $forceLock = Enter-CdrIntentLock -RepoRoot $fullRoot
        try {
            $curIntent = Get-CdrComponentIntent -RepoRoot $fullRoot -Component $Component
            $curJournal = $null
            try {
                $curJournal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component
            } catch {
                $forceReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = 'unknown'
                    Stopped         = $false
                    WasRunning      = $true
                    ProcessId       = [int]$live.ProcessId
                    ProcessIdentity = [string]$live.ProcessIdentity
                    Reason          = $_.Exception.Message
                }
            }
            if ($null -eq $forceReturn -and $null -ne $curJournal -and $curJournal.Phase -in @('launching', 'launch_outcome_unknown')) {
                $forceReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = 'unknown'
                    Stopped         = $false
                    WasRunning      = $true
                    ProcessId       = [int]$live.ProcessId
                    ProcessIdentity = [string]$live.ProcessIdentity
                    Reason          = "launch_outcome_unknown:$($curJournal.Phase)"
                }
            }
            if ($null -eq $forceReturn) {
                $intentStillCurrent = ([uint64]$curIntent.Revision -eq $stopOperationRevision -and [string]$curIntent.DesiredState -eq $stopOperationDesiredState)
                $journalStillMatches = ($null -eq $curJournal) -or
                    ($curJournal.Phase -notin @('launching', 'launch_outcome_unknown') -and
                     -not [string]::IsNullOrWhiteSpace($curJournal.ProcessIdentity) -and
                     [string]$curJournal.ProcessIdentity -ceq [string]$live.ProcessIdentity -and
                     [uint64]$curJournal.IntentRevision -le $stopOperationRevision)
                if (-not $intentStillCurrent -or -not $journalStillMatches) {
                    $forceReturn = [pscustomobject]@{
                        Component       = $Component
                        State           = 'still_running'
                        Stopped         = $false
                        WasRunning      = $true
                        ProcessId       = [int]$live.ProcessId
                        ProcessIdentity = [string]$live.ProcessIdentity
                        Reason          = 'superseded_by_newer_intent'
                    }
                } else {
                    $forceCommitted = $true
                    if ($null -ne $KillOverride) {
                        & $KillOverride $live
                    } else {
                        Stop-CdrVerifiedProcess -ProcessId $live.ProcessId -ExpectedIdentity $live.ProcessIdentity -ExpectedBinaryPath $live.BinaryPath -Process $live.Process -WaitTimeoutMs 0 | Out-Null
                    }
                }
            }
        } finally {
            Exit-CdrIntentLock $forceLock
        }
        if ($null -ne $forceReturn) {
            return $forceReturn
        }
        if ($forceCommitted -and $null -eq $KillOverride) {
            $killDeadline = [DateTime]::UtcNow.AddMilliseconds(2000)
            while ([DateTime]::UtcNow -lt $killDeadline) {
                $killCheck = Get-CdrProcessObservation -ProcessId $live.ProcessId -ExpectedIdentity $live.ProcessIdentity -ExpectedBinaryPath $live.BinaryPath
                if ($killCheck.State -eq 'stopped') {
                    break
                }
                Start-Sleep -Milliseconds 50
            }
        }
        $postWait = Get-CdrProcessObservation -ProcessId $live.ProcessId -ExpectedIdentity $live.ProcessIdentity -ExpectedBinaryPath $live.BinaryPath
    }

    $stoppedCleanly = ($postWait.State -eq 'stopped')
    if ($stoppedCleanly) {
        $supersededReturn = $null
        $compLock = Enter-CdrIntentLock -RepoRoot $fullRoot
        try {
            $intent = Get-CdrComponentIntent -RepoRoot $fullRoot -Component $Component
            $intentStillCurrent = ([uint64]$intent.Revision -eq $stopOperationRevision -and [string]$intent.DesiredState -eq $stopOperationDesiredState)
            if (-not $intentStillCurrent) {
                $curObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component $Component -BinaryPath $BinaryPath
                $supersededReturn = [pscustomobject]@{
                    Component       = $Component
                    State           = if ($curObs.State -eq 'running') { 'still_running' } else { [string]$curObs.State }
                    Stopped         = $false
                    WasRunning      = $true
                    ProcessId       = [int]$curObs.ProcessId
                    ProcessIdentity = [string]$curObs.ProcessIdentity
                    Reason          = 'superseded_by_newer_intent'
                }
            } else {
                if (-not $PreserveIntent -and $intent.DesiredState -eq 'stopped') {
                    Set-CdrManualStopMarker -RepoRoot $fullRoot -Component $Component -Reason $Source
                }
                if ($hasControlAuthority) {
                    $curJournal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component
                    $journalMatchesTarget = ($null -eq $curJournal) -or
                        ($curJournal.Phase -notin @('launching', 'launch_outcome_unknown') -and
                         -not [string]::IsNullOrWhiteSpace($curJournal.ProcessIdentity) -and
                         [string]$curJournal.ProcessIdentity -ceq [string]$live.ProcessIdentity -and
                         [uint64]$curJournal.IntentRevision -le $stopOperationRevision)
                    if ($journalMatchesTarget) {
                        Write-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $Component `
                            -Phase 'stopped' -BinaryPath $live.BinaryPath -Action 'stop' `
                            -IntentRevision $stopOperationRevision -ProcessId $live.ProcessId -ProcessIdentity $live.ProcessIdentity | Out-Null
                    }
                }
            }
        } finally {
            Exit-CdrIntentLock $compLock
        }
        if ($null -ne $supersededReturn) {
            return $supersededReturn
        }
    }

    return [pscustomobject]@{
        Component       = $Component
        State           = if ($stoppedCleanly) { 'stopped' } elseif ($postWait.State -eq 'unknown') { 'unknown' } else { 'still_running' }
        Stopped         = $stoppedCleanly
        WasRunning      = $true
        ProcessId       = [int]$live.ProcessId
        ProcessIdentity = [string]$live.ProcessIdentity
        Reason          = [string]$postWait.Reason
    }
}

function Copy-CdrAtomicBinary {
    param(
        [Parameter(Mandatory)][string]$SourcePath,
        [Parameter(Mandatory)][string]$DestinationPath
    )
    $parent = Split-Path -Parent $DestinationPath
    if ($parent -and -not [IO.Directory]::Exists($parent)) {
        [IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    $expectedHash = (Get-FileHash -LiteralPath $SourcePath -Algorithm SHA256).Hash
    $temp = "$DestinationPath.stage.$PID.$([guid]::NewGuid().ToString('N'))"
    try {
        [IO.File]::Copy($SourcePath, $temp, $false)
        if ((Get-FileHash -LiteralPath $temp -Algorithm SHA256).Hash -cne $expectedHash) {
            throw "Binary hash changed during staging for $DestinationPath"
        }
        if ([IO.File]::Exists($DestinationPath)) {
            [IO.File]::Replace($temp, $DestinationPath, [NullString]::Value)
        } else {
            [IO.File]::Move($temp, $DestinationPath)
        }
    } finally {
        if ([IO.File]::Exists($temp)) {
            [IO.File]::Delete($temp)
        }
    }
}

function Update-CdrBotBinaryOnly {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [Parameter(Mandatory)][string]$CandidateBotBinary,
        [string]$CandidateMcpBinary = '',
        [string]$CandidateScriptsDir = '',
        [string]$InstalledBotBinary = '',
        [string]$InstalledMcpBinary = '',
        [string[]]$BotLaunchArguments = @(),
        [int]$GraceTimeoutMs = 45000,
        [switch]$AllowForceKillBot,
        [switch]$SkipBotRestart
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $targetBot = if ([string]::IsNullOrWhiteSpace($InstalledBotBinary)) {
        Join-Path $fullRoot 'target\release\cdr-runtime.exe'
    } else {
        [IO.Path]::GetFullPath($InstalledBotBinary)
    }
    $targetMcp = if ([string]::IsNullOrWhiteSpace($InstalledMcpBinary)) {
        Join-Path $fullRoot 'target\release\cdr-remote-agent.exe'
    } else {
        [IO.Path]::GetFullPath($InstalledMcpBinary)
    }

    $mcpObsBefore = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $targetMcp
    if ($mcpObsBefore.State -eq 'unknown') {
        throw "Bot-only update refused: external MCP observation is unknown ($($mcpObsBefore.Reason))"
    }
    $liveMcpBefore = if ($mcpObsBefore.State -eq 'running') { $mcpObsBefore } else { $null }

    $botIntentBefore = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'bot'
    $stopBot = Stop-CdrComponentRuntime -RepoRoot $fullRoot -Component 'bot' -BinaryPath $targetBot `
        -GraceTimeoutMs $GraceTimeoutMs -Source 'bot-only-update' -PreserveIntent -AllowForceKill:$AllowForceKillBot
    if (-not $stopBot.Stopped) {
        throw "Bot-only update refused: running bot did not stop cleanly (state=$($stopBot.State))"
    }

    Copy-CdrAtomicBinary -SourcePath ([IO.Path]::GetFullPath($CandidateBotBinary)) -DestinationPath $targetBot

    $mcpStaged = $false
    $stagedMcpPath = Join-Path $fullRoot '.cdr-maintenance\staged-cdr-remote-agent.exe'
    if (-not [string]::IsNullOrWhiteSpace($CandidateMcpBinary) -and [IO.File]::Exists($CandidateMcpBinary)) {
        if ($null -ne $liveMcpBefore) {
            # External MCP is actively running: never stop or overwrite its executable or helper scripts during a bot-only update!
            Copy-CdrAtomicBinary -SourcePath ([IO.Path]::GetFullPath($CandidateMcpBinary)) -DestinationPath $stagedMcpPath
            $mcpStaged = $true
        } else {
            Copy-CdrAtomicBinary -SourcePath ([IO.Path]::GetFullPath($CandidateMcpBinary)) -DestinationPath $targetMcp
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($CandidateScriptsDir) -and [IO.Directory]::Exists($CandidateScriptsDir) -and $null -ne $liveMcpBefore) {
        $stagedScriptsDir = Join-Path $fullRoot '.cdr-maintenance\staged-scripts'
        if (-not [IO.Directory]::Exists($stagedScriptsDir)) {
            [IO.Directory]::CreateDirectory($stagedScriptsDir) | Out-Null
        }
        foreach ($scriptFile in [IO.Directory]::GetFiles($CandidateScriptsDir)) {
            $destFile = Join-Path $stagedScriptsDir ([IO.Path]::GetFileName($scriptFile))
            Copy-CdrAtomicBinary -SourcePath $scriptFile -DestinationPath $destFile
        }
    }

    $replacementBot = $null
    if (-not $SkipBotRestart -and $botIntentBefore.DesiredState -eq 'running') {
        $botIntentAfter = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'bot'
        if ($botIntentAfter.Revision -eq $botIntentBefore.Revision -and $botIntentAfter.DesiredState -eq 'running') {
            Clear-CdrManualStopMarker -RepoRoot $fullRoot -Component 'bot'
            $replacementBot = Invoke-CdrJournaledProcessLaunch `
                -RepoRoot $fullRoot `
                -Component 'bot' `
                -FilePath $targetBot `
                -ArgumentList $BotLaunchArguments `
                -WorkingDirectory $fullRoot `
                -TargetBinaryPath $targetBot `
                -Action 'bot-only-update-restart' `
                -ExpectedIntentRevision $botIntentBefore.Revision
        }
    }

    $mcpInterrupted = $false
    if ($null -ne $liveMcpBefore) {
        $mcpInterrupted = -not (Test-CdrProcessMatchesIdentity `
            -ProcessId $liveMcpBefore.ProcessId `
            -ExpectedIdentity $liveMcpBefore.ProcessIdentity `
            -ExpectedBinaryPath $liveMcpBefore.BinaryPath)
    }

    return [pscustomobject]@{
        BotUpdated              = $true
        McpInterrupted          = $mcpInterrupted
        McpStagedForNextRestart = $mcpStaged
        StagedMcpPath           = if ($mcpStaged) { $stagedMcpPath } else { '' }
        ReplacementBot          = $replacementBot
    }
}

function Read-CdrHealthyDrainIdentity {
    param([Parameter(Mandatory)][string]$Path)
    if (-not [IO.File]::Exists($Path)) { return $null }
    $text = [IO.File]::ReadAllText($Path, [Text.UTF8Encoding]::UTF8)
    if ($text.Length -gt 4096) { throw 'drain_identity_exceeds_bound' }
    $fields = @{}
    foreach ($line in ($text -split "`r?`n")) {
        if ([string]::IsNullOrEmpty($line)) { continue }
        $parts = $line -split '=', 2
        if ($parts.Count -ne 2 -or [string]::IsNullOrEmpty($parts[0]) -or [string]::IsNullOrEmpty($parts[1]) -or $fields.ContainsKey($parts[0])) {
            throw 'drain_identity_malformed'
        }
        $fields[$parts[0]] = $parts[1]
    }
    foreach ($req in @('version', 'runtime_id', 'pid', 'state')) {
        if (-not $fields.ContainsKey($req)) {
            throw "drain_identity_missing_$req"
        }
    }
    if ($fields['version'] -cne '1') { throw 'drain_identity_version_unsupported' }
    if ($fields['state'] -cne 'open') { throw "drain_identity_state_not_open:$($fields['state'])" }
    $pidVal = 0
    if (-not [int]::TryParse($fields['pid'], [ref]$pidVal) -or $pidVal -le 0) {
        throw 'drain_identity_pid_invalid'
    }
    return [pscustomobject]@{
        RuntimeId = [string]$fields['runtime_id']
        Pid       = $pidVal
        State     = [string]$fields['state']
    }
}

function Get-CdrActiveLegacyHelpers {
    param([Parameter(Mandatory)][Alias('Root')][string]$RepoRoot)
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
    $rootNorm = $fullRoot.Replace('\', '/').ToLowerInvariant()
    $helperNames = @(
        'codex-discord-bot.cmd',
        'codex-discord-bot-headless.vbs',
        'codex-discord-watchdog.ps1',
        'codex-discord-rust-watchdog.ps1',
        'codex-discord-rust-restart.ps1'
    )
    $allProcs = $null
    try {
        $allProcs = @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop)
    } catch {
        return @([pscustomobject]@{
            ProcessId   = -1
            HelperName  = 'win32_process_observation_failed'
            CommandLine = $_.Exception.Message
            Uncertain   = $true
        })
    }
    if ($null -eq $allProcs -or $allProcs.Count -eq 0) {
        return @([pscustomobject]@{
            ProcessId   = -1
            HelperName  = 'win32_process_inventory_empty'
            CommandLine = ''
            Uncertain   = $true
        })
    }
    $currentSessionId = -1
    try {
        $currentSessionId = [int](Get-Process -Id $PID -ErrorAction Stop).SessionId
    } catch {
        $currentSessionId = -1
    }
    $testProcessAliveOrUncertain = {
        param([int]$TargetPid)
        try {
            $liveProc = Get-Process -Id $TargetPid -ErrorAction Stop
            if ($null -eq $liveProc -or ($liveProc.PSObject.Properties['HasExited'] -and $liveProc.HasExited)) {
                return $false
            }
            return $true
        } catch {
            if ($_.FullyQualifiedErrorId -like 'NoProcessFoundForGivenId*' -or
                $_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound -or
                $_.Exception.Message -match 'Cannot find a process with the process identifier') {
                return $false
            }
            return $true
        }
    }
    $active = New-Object System.Collections.Generic.List[object]
    foreach ($p in $allProcs) {
        if ($null -eq $p -or -not $p.ProcessId) { continue }
        $procId = [int]$p.ProcessId
        if ($procId -eq [int]$PID) { continue }
        $cmdLine = [string]$p.CommandLine
        if ([string]::IsNullOrWhiteSpace($cmdLine)) {
            $pName = ([string]$p.Name).ToLowerInvariant()
            if ($pName -in @('cmd.exe', 'wscript.exe', 'cscript.exe', 'powershell.exe', 'pwsh.exe') -and
                ($currentSessionId -lt 0 -or [int]$p.SessionId -eq $currentSessionId)) {
                if (& $testProcessAliveOrUncertain $procId) {
                    $active.Add([pscustomobject]@{
                        ProcessId   = $procId
                        HelperName  = 'unknown_commandline'
                        CommandLine = ''
                        Uncertain   = $true
                    })
                }
            }
            continue
        }
        $cmdNorm = $cmdLine.Replace('\', '/').ToLowerInvariant()
        foreach ($hName in $helperNames) {
            $hLow = $hName.ToLowerInvariant()
            if (-not $cmdNorm.Contains($hLow)) { continue }
            $targetsThisRootOrAmbiguous = $false
            if ($cmdNorm.Contains($rootNorm)) {
                $targetsThisRootOrAmbiguous = $true
            } else {
                $tokenMatches = [regex]::Matches($cmdNorm, "(?i)(?:^|[\s`" '=])([^\s`"']*?)$([regex]::Escape($hLow))(?=$|[\s`"'])")
                if ($tokenMatches.Count -eq 0) {
                    $targetsThisRootOrAmbiguous = $true
                } else {
                    foreach ($m in $tokenMatches) {
                        $prefix = $m.Groups[1].Value.Trim()
                        if ([string]::IsNullOrEmpty($prefix) -or $prefix -notmatch '^(?:[a-z]:/|//)') {
                            $targetsThisRootOrAmbiguous = $true
                            break
                        }
                    }
                }
                if (-not $targetsThisRootOrAmbiguous -and $cmdNorm -match '-(?:reporoot|root)\s+["'']?(?!(?:[a-z]:/|//))') {
                    $targetsThisRootOrAmbiguous = $true
                }
            }
            if ($targetsThisRootOrAmbiguous -and (& $testProcessAliveOrUncertain $procId)) {
                $active.Add([pscustomobject]@{
                    ProcessId   = $procId
                    HelperName  = $hName
                    CommandLine = $cmdLine
                })
                break
            }
        }
    }
    return $active.ToArray()
}

function Wait-CdrLegacyHelpersExited {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [int]$TimeoutMs = 2000
    )
    $deadline = [DateTime]::UtcNow.AddMilliseconds([Math]::Max(0, $TimeoutMs))
    while ($true) {
        $active = @(Get-CdrActiveLegacyHelpers -RepoRoot $RepoRoot)
        if ($active.Count -eq 0) {
            return [pscustomobject]@{ AllExited = $true; Active = @() }
        }
        if ([DateTime]::UtcNow -ge $deadline) {
            return [pscustomobject]@{ AllExited = $false; Active = $active }
        }
        Start-Sleep -Milliseconds 50
    }
}

function Test-CdrExternalMcpCutoverStatusReady {
    param(
        [Parameter(Mandatory)][string]$StatusPath,
        [Parameter(Mandatory)]$McpPaths,
        [Parameter(Mandatory)][int]$ExpectedPid,
        [Parameter(Mandatory)][string]$ExpectedIdentity,
        [Parameter(Mandatory)][string]$ExpectedBinaryPath,
        [string]$ExpectedArtifactHash = ''
    )
    if (-not $McpPaths.Enabled -or
        [string]::IsNullOrWhiteSpace($McpPaths.DeviceId) -or
        [string]::IsNullOrWhiteSpace($McpPaths.BridgeFingerprint)) {
        return $false
    }
    if (-not [IO.File]::Exists($StatusPath) -or [IO.Directory]::Exists($StatusPath)) {
        return $false
    }
    $record = $null
    try {
        $record = [IO.File]::ReadAllText($StatusPath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
    } catch {
        return $false
    }
    if ($null -eq $record) { return $false }

    foreach ($reqProp in @(
        'schema_version', 'component', 'install_root', 'pid', 'process_identity',
        'executable_path', 'artifact_sha256', 'build_info', 'updated_at_ms',
        'heartbeat_at_ms', 'device_id', 'bridge_fingerprint', 'hello_ack',
        'ready', 'draining', 'generation'
    )) {
        if (-not $record.PSObject.Properties[$reqProp] -or $null -eq $record.$reqProp) {
            return $false
        }
    }

    if (($record.schema_version -isnot [int] -and $record.schema_version -isnot [long]) -or [int]$record.schema_version -ne 1) {
        return $false
    }
    if ($record.component -isnot [string] -or [string]$record.component -cne 'mcp') {
        return $false
    }
    if ($record.install_root -isnot [string] -or [string]::IsNullOrWhiteSpace($record.install_root)) {
        return $false
    }
    $recRoot = ([string]$record.install_root).Trim().TrimEnd('/', '\').Replace('\', '/').ToLowerInvariant()
    if ($recRoot -cne $McpPaths.NormalizedRoot) {
        return $false
    }
    if (($record.pid -isnot [int] -and $record.pid -isnot [long]) -or [int]$record.pid -ne $ExpectedPid) {
        return $false
    }
    if ($record.process_identity -isnot [string] -or [string]$record.process_identity -cne $ExpectedIdentity) {
        return $false
    }
    if ($record.executable_path -isnot [string] -or [string]::IsNullOrWhiteSpace($record.executable_path)) {
        return $false
    }
    if (-not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath([string]$record.executable_path), [IO.Path]::GetFullPath($ExpectedBinaryPath))) {
        return $false
    }
    if ($record.artifact_sha256 -isnot [string] -or $record.artifact_sha256 -notmatch '^[0-9a-fA-F]{64}$') {
        return $false
    }
    if (-not [IO.File]::Exists($ExpectedBinaryPath)) {
        return $false
    }
    $actualHash = (Get-FileHash -LiteralPath $ExpectedBinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -cne ([string]$record.artifact_sha256).ToLowerInvariant()) {
        return $false
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedArtifactHash) -and
        $ExpectedArtifactHash.ToLowerInvariant() -cne ([string]$record.artifact_sha256).ToLowerInvariant()) {
        return $false
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
        return $false
    }
    if (($record.updated_at_ms -isnot [int] -and $record.updated_at_ms -isnot [long]) -or
        ($record.heartbeat_at_ms -isnot [int] -and $record.heartbeat_at_ms -isnot [long])) {
        return $false
    }
    if ($record.device_id -isnot [string] -or [string]$record.device_id -cne $McpPaths.DeviceId) {
        return $false
    }
    if ($record.bridge_fingerprint -isnot [string] -or [string]$record.bridge_fingerprint -cne $McpPaths.BridgeFingerprint) {
        return $false
    }
    if (($record.generation -isnot [int] -and $record.generation -isnot [long]) -or [int64]$record.generation -le 0) {
        return $false
    }
    # Strict boolean checks: never coerce string "false" to $true via [bool]
    if ($record.ready -isnot [bool] -or $record.hello_ack -isnot [bool] -or $record.draining -isnot [bool]) {
        return $false
    }
    if (-not ($record.ready -eq $true -and $record.hello_ack -eq $true -and $record.draining -eq $false)) {
        return $false
    }
    $nowMs = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $ageMs = $nowMs - [int64]$record.updated_at_ms
    $hbAgeMs = $nowMs - [int64]$record.heartbeat_at_ms
    if ([int64]$record.updated_at_ms -le 0 -or [int64]$record.heartbeat_at_ms -le 0 -or
        $ageMs -lt -2000 -or $hbAgeMs -lt -2000 -or
        ([int64]$record.updated_at_ms -gt ([int64]$record.heartbeat_at_ms + 2000)) -or
        $ageMs -gt 6000 -or $hbAgeMs -gt 6000) {
        return $false
    }
    if (-not $record.PSObject.Properties['connection_verified_at_ms'] -or $null -eq $record.connection_verified_at_ms) {
        return $false
    }
    if ($record.connection_verified_at_ms -isnot [int] -and $record.connection_verified_at_ms -isnot [long]) {
        return $false
    }
    $connAgeMs = $nowMs - [int64]$record.connection_verified_at_ms
    if ([int64]$record.connection_verified_at_ms -le 0 -or $connAgeMs -lt -2000 -or
        ([int64]$record.connection_verified_at_ms -gt ([int64]$record.heartbeat_at_ms + 2000)) -or
        $connAgeMs -gt 6000) {
        return $false
    }
    return $true
}

function Publish-CdrCutoverDrainFenceMarkers {
    param(
        [Parameter(Mandatory)][string]$RestartMarkerPath,
        [Parameter(Mandatory)][string]$DrainPreparePath,
        [Parameter(Mandatory)][string]$RuntimeId,
        [Parameter(Mandatory)][string]$ProcessIdentity,
        [Parameter(Mandatory)][string]$Nonce,
        [scriptblock]$OnBeforeMarkerPublish = $null
    )
    if (-not (Get-Command Write-AtomicRestartMarker -ErrorAction SilentlyContinue)) {
        $drainCandidates = @(
            (Join-Path (Split-Path -Parent $RestartMarkerPath) 'codex-discord-rust-drain.ps1'),
            (Join-Path (Split-Path -Parent $PSScriptRoot) 'codex-discord-rust-drain.ps1'),
            (Join-Path $PSScriptRoot 'codex-discord-rust-drain.ps1')
        )
        $drainScript = $drainCandidates | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and [IO.File]::Exists($_) } | Select-Object -First 1
        if ([string]::IsNullOrWhiteSpace($drainScript)) {
            throw 'Canonical drain helper script codex-discord-rust-drain.ps1 not found for atomic marker publication'
        }
        . $drainScript
    }
    $fencePayload = "version=1`nruntime_id=$RuntimeId`nprocess_identity=$ProcessIdentity`nnonce=$Nonce`n"
    Write-AtomicRestartMarker -Path $RestartMarkerPath -Text $fencePayload -OnBeforePublish $OnBeforeMarkerPublish
    Write-AtomicRestartMarker -Path $DrainPreparePath -Text $fencePayload -OnBeforePublish $OnBeforeMarkerPublish
}

function Invoke-CdrMcpOwnershipCutover {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [string]$McpBinaryPath = '',
        [string]$BotBinaryPath = '',
        [string[]]$McpArguments = @(),
        [string]$ExpectedEmbeddedIdentity = '',
        [int]$EmbeddedExitTimeoutMs = 45000,
        [int]$LegacyHelperExitTimeoutMs = 2000,
        [int]$ReadinessTimeoutMs = 10000,
        [switch]$AllowClearManualStop,
        [scriptblock]$OnBeforeDrainMarkerPublish = $null,
        [scriptblock]$OnBeforeCommitWrite = $null,
        [scriptblock]$OnAfterCommitBeforeFenceRelease = $null
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $resolvedMcpBin = if (-not [string]::IsNullOrWhiteSpace($McpBinaryPath)) {
        [IO.Path]::GetFullPath($McpBinaryPath)
    } else {
        $cands = Resolve-CdrComponentBinaryCandidates -RepoRoot $fullRoot -Component 'mcp'
        if ($cands.Count -eq 0) { throw "External MCP binary not found under $fullRoot" }
        $cands[0]
    }
    $statePath = Join-Path $fullRoot '.cdr-maintenance\mcp-ownership-state.json'
    $cutoverMarkerPath = Join-Path $fullRoot '.codex_discord_runtime.cutover'
    $disabledFencePath = Join-Path $fullRoot '.codex_discord_bot.disabled'
    $drainIdentityPath = Join-Path $fullRoot '.codex_discord_rust.drain.identity'
    $drainPreparePath = Join-Path $fullRoot '.codex_discord_rust.drain.prepare'
    $drainAckPath = Join-Path $fullRoot '.codex_discord_rust.drain.ack'
    $restartMarkerPath = Join-Path $fullRoot '.codex_discord_rust.restart'

    $priorState = $null
    if ([IO.File]::Exists($statePath)) {
        try {
            $priorState = [IO.File]::ReadAllText($statePath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
        } catch {
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                reason = 'corrupt_prior_ownership_state'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
            throw "Cutover held: existing ownership state at $statePath is corrupt"
        }
    }

    $priorEmbeddedIdentity = if ($null -ne $priorState -and $priorState.PSObject.Properties['embedded_owner_identity']) {
        [string]$priorState.embedded_owner_identity
    } else { '' }
    $priorEmbeddedDrainRuntimeId = if ($null -ne $priorState -and $priorState.PSObject.Properties['embedded_drain_runtime_id']) {
        [string]$priorState.embedded_drain_runtime_id
    } else { '' }
    $priorEmbeddedExited = [bool]($null -ne $priorState -and $priorState.PSObject.Properties['embedded_owner_exited'] -and $priorState.embedded_owner_exited)

    # 1. Fence old launch authorities / delayed helpers under legacy control lock, legacy .disabled marker, and component locks
    $forceLockPath = Join-Path $fullRoot '.codex_discord_rust.force.lock'
    if ([IO.File]::Exists($forceLockPath)) {
        $forceProbe = $null
        try {
            $forceProbe = [IO.File]::Open($forceLockPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        } catch [IO.IOException] {
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                embedded_owner_identity = $priorEmbeddedIdentity; embedded_drain_runtime_id = $priorEmbeddedDrainRuntimeId; embedded_owner_exited = $priorEmbeddedExited
                reason = 'emergency_force_restart_active'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
            throw 'Cutover held: emergency force restart owns control lock'
        } finally {
            if ($null -ne $forceProbe) { $forceProbe.Dispose() }
        }
    }

    $legacyControlLockPath = Join-Path $fullRoot '.codex_discord_rust.control.lock'
    $legacyControlHandle = $null
    try {
        try {
            $legacyControlHandle = [IO.File]::Open($legacyControlLockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
            $curProc = [Diagnostics.Process]::GetCurrentProcess()
            try {
                $curTicks = $curProc.StartTime.ToUniversalTime().Ticks
                $curTicks -= ($curTicks % 10)
                $ctrlRec = @{
                    Version = 1; Root = $fullRoot; ProcessId = $PID; StartTicks = "$curTicks"
                    Executable = $curProc.MainModule.FileName; Purpose = 'mcp_cutover'
                } | ConvertTo-Json -Compress
                $ctrlBytes = [Text.UTF8Encoding]::new($false).GetBytes($ctrlRec)
                $legacyControlHandle.SetLength(0)
                $legacyControlHandle.Write($ctrlBytes, 0, $ctrlBytes.Length)
                $legacyControlHandle.Flush()
            } finally {
                $curProc.Dispose()
            }
        } catch [IO.IOException] {
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                embedded_owner_identity = $priorEmbeddedIdentity; embedded_drain_runtime_id = $priorEmbeddedDrainRuntimeId; embedded_owner_exited = $priorEmbeddedExited
                reason = 'legacy_control_lock_busy'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
            throw 'Cutover held: legacy control lock is owned by another operation'
        }

        $botObs = $null
        $mcpObs = $null
        $embeddedTargetIdentity = ''
        $embeddedDrainRuntimeId = $priorEmbeddedDrainRuntimeId
        $embeddedExited = $false
        $authorizedMcpRevision = [uint64]0
        $resumedCommittedResult = $null

        $lock = Enter-CdrComponentShortLock -RepoRoot $fullRoot -Component 'common'
        try {
            # Write legacy .codex_discord_bot.disabled fence understood by old no-argument CMD and old watchdog/restart helpers
            if (-not [IO.File]::Exists($disabledFencePath)) {
                [IO.File]::WriteAllText($disabledFencePath, 'mcp_cutover_fence', [Text.UTF8Encoding]::new($false))
            }
            [IO.File]::WriteAllText($cutoverMarkerPath, "cutover_pending`n", [Text.UTF8Encoding]::new($false))

            # Prove any already-admitted supported old helpers targeting this install root have exited while fenced
            $helperWait = $null
            try {
                $helperWait = Wait-CdrLegacyHelpersExited -RepoRoot $fullRoot -TimeoutMs $LegacyHelperExitTimeoutMs
            } catch {
                Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                    schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                    embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $priorEmbeddedExited
                    reason = "legacy_helper_observation_failed:$($_.Exception.Message)"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                })
                throw "Cutover held: could not verify legacy helper completion ($($_.Exception.Message))"
            }
            if (-not $helperWait.AllExited) {
                $firstHelper = $helperWait.Active[0]
                Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                    schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                    embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $priorEmbeddedExited
                    reason = "legacy_helper_still_active:$($firstHelper.HelperName):$($firstHelper.ProcessId)"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                })
                throw "Cutover held: already-admitted legacy helper $($firstHelper.HelperName) (PID $($firstHelper.ProcessId)) is still active"
            }

            foreach ($marker in @(
                '.codex_discord_rust.restart',
                '.codex_discord_rust.restart.launch',
                '.codex_discord_rust.force.launch',
                '.codex_discord_rust.maintenance.v2',
                '.codex_discord_rust.drain.prepare',
                '.codex_discord_rust.drain.ack'
            )) {
                if ([IO.File]::Exists((Join-Path $fullRoot $marker))) {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                        embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $priorEmbeddedExited
                        reason = "pending_legacy_operation:$marker"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw "Cutover held: pending legacy maintenance/restart marker $marker is active"
                }
            }
            if ([IO.Directory]::Exists($fullRoot) -and [IO.Directory]::GetFiles($fullRoot, '.codex_discord_rust.restart.claimed.*').Count -gt 0) {
                Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                    schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                    embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $priorEmbeddedExited
                    reason = 'pending_legacy_restart_claim'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                })
                throw 'Cutover held: orphan legacy restart claim is active'
            }
            foreach ($comp in @('bot', 'mcp')) {
                $compJournal = $null
                try {
                    $compJournal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component $comp
                } catch {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                        embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $priorEmbeddedExited
                        reason = "journal_corrupt:$comp"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw
                }
                if ($null -ne $compJournal -and $compJournal.Phase -in @('launching', 'launch_outcome_unknown')) {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                        embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $priorEmbeddedExited
                        reason = "journal_uncertain:${comp}:$($compJournal.Phase)"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw "Cutover held: $comp launch journal is in $($compJournal.Phase) phase"
                }
            }

            $botObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'bot' -BinaryPath $BotBinaryPath
            $mcpObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $resolvedMcpBin
            if ($botObs.State -eq 'unknown' -or $mcpObs.State -eq 'unknown') {
                Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                    schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                    embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $priorEmbeddedExited
                    reason = "observation_unknown:bot=$($botObs.State),mcp=$($mcpObs.State)"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                })
                throw "Cutover held: component observation is unknown (bot=$($botObs.Reason), mcp=$($mcpObs.Reason))"
            }

            # R4 Idempotent resume: If ownership state was already atomically committed before an interruption
            # prevented self-owned fence cleanup, re-verify the exact recorded external owner and its live connection
            # and finish only self-owned fence cleanup without spawning a duplicate external MCP.
            if ($null -ne $priorState -and
                $priorState.PSObject.Properties['schema_version'] -and [int]$priorState.schema_version -eq 1 -and
                $priorState.PSObject.Properties['mode'] -and [string]$priorState.mode -ceq 'external' -and
                $priorState.PSObject.Properties['state'] -and [string]$priorState.state -ceq 'committed' -and
                $priorState.PSObject.Properties['committed'] -and $priorState.committed -is [bool] -and $priorState.committed -eq $true) {
                $committedOwnerPid = if ($priorState.PSObject.Properties['owner_pid'] -and ($priorState.owner_pid -is [int] -or $priorState.owner_pid -is [long])) { [int]$priorState.owner_pid } else { 0 }
                $committedOwnerIdentity = if ($priorState.PSObject.Properties['owner_identity'] -and $priorState.owner_identity -is [string]) { [string]$priorState.owner_identity } else { '' }
                if ($committedOwnerPid -le 0 -or $committedOwnerIdentity -notmatch '^(\d+)\|(\d+)$' -or [int]$Matches[1] -ne $committedOwnerPid) {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                        embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $priorEmbeddedExited
                        reason = 'committed_owner_identity_invalid'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw 'Cutover held: committed ownership state has missing or invalid owner_identity'
                }
                if (-not [string]::IsNullOrWhiteSpace($priorEmbeddedIdentity) -and $priorEmbeddedIdentity -ne 'none') {
                    if ($priorEmbeddedIdentity -notmatch '^(\d+)\|(\d+)$') {
                        Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                            schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                            embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $false
                            reason = 'committed_embedded_identity_invalid'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                        })
                        throw 'Cutover held: committed ownership state has invalid embedded_owner_identity'
                    }
                    $priorEmbedPid = [int]$Matches[1]
                    $priorEmbedObs = Get-CdrProcessObservation -ProcessId $priorEmbedPid -ExpectedIdentity $priorEmbeddedIdentity -ExpectedBinaryPath $BotBinaryPath
                    if ($priorEmbedObs.State -ne 'stopped') {
                        Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                            schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                            embedded_owner_identity = $priorEmbeddedIdentity; embedded_owner_exited = $false
                            reason = "committed_embedded_owner_not_stopped:$($priorEmbedObs.State)"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                        })
                        throw "Cutover held: prior embedded owner $priorEmbeddedIdentity is still $($priorEmbedObs.State)"
                    }
                }
                if ($mcpObs.State -eq 'running' -and
                    [int]$mcpObs.ProcessId -eq $committedOwnerPid -and
                    [string]$mcpObs.ProcessIdentity -ceq $committedOwnerIdentity) {
                    $mcpIntentNow = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp'
                    if ($mcpIntentNow.DesiredState -eq 'stopped' -or (Test-CdrManualStopRequested -RepoRoot $fullRoot -Component 'mcp')) {
                        Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                            schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                            embedded_owner_identity = $(if ($priorEmbeddedIdentity) { $priorEmbeddedIdentity } else { 'none' })
                            embedded_owner_exited = $priorEmbeddedExited
                            reason = 'mcp_manual_stop_active'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                        })
                        throw 'Cutover held: MCP manual stop is active while resuming committed state'
                    }
                    $resumeMcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
                    $resumeStatusCands = @($resumeMcpPaths.StatusPath, (Join-Path $fullRoot 'bot_data\remote_mcp.status.json')) | Select-Object -Unique
                    $resumeJournal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component 'mcp'
                    $resumeExpectedHash = if ($null -ne $resumeJournal -and $resumeJournal.PSObject.Properties['ArtifactHash']) { [string]$resumeJournal.ArtifactHash } else { '' }
                    $resumeReady = $false
                    if ([IO.File]::Exists($resumeMcpPaths.InstanceLockPath)) {
                        try {
                            $resumeLockRec = Read-CdrMcpInstanceLockRecord -LockPath $resumeMcpPaths.InstanceLockPath
                            if ($null -ne $resumeLockRec -and
                                [int]$resumeLockRec.ProcessId -eq $committedOwnerPid -and
                                [string]$resumeLockRec.ProcessIdentity -ceq $committedOwnerIdentity) {
                                foreach ($stCand in $resumeStatusCands) {
                                    if (Test-CdrExternalMcpCutoverStatusReady `
                                        -StatusPath $stCand `
                                        -McpPaths $resumeMcpPaths `
                                        -ExpectedPid $committedOwnerPid `
                                        -ExpectedIdentity $committedOwnerIdentity `
                                        -ExpectedBinaryPath $resolvedMcpBin `
                                        -ExpectedArtifactHash $resumeExpectedHash) {
                                        $resumeReady = $true
                                        break
                                    }
                                }
                            }
                        } catch { }
                    }
                    if (-not $resumeReady) {
                        Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                            schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                            embedded_owner_identity = $(if ($priorEmbeddedIdentity) { $priorEmbeddedIdentity } else { 'none' })
                            embedded_owner_exited = $priorEmbeddedExited
                            reason = "committed_owner_not_ready:$committedOwnerIdentity"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                        })
                        throw "Cutover held: committed external owner $committedOwnerIdentity failed live status verification"
                    }
                    if ([IO.File]::Exists($cutoverMarkerPath)) {
                        [IO.File]::Delete($cutoverMarkerPath)
                    }
                    if ([IO.File]::Exists($disabledFencePath) -and ([IO.File]::ReadAllText($disabledFencePath).Trim() -ceq 'mcp_cutover_fence')) {
                        [IO.File]::Delete($disabledFencePath)
                    }
                    $resumedCommittedResult = [pscustomobject]@{
                        Committed            = $true
                        Mode                 = 'external'
                        Process              = $mcpObs.Process
                        ProcessId            = $committedOwnerPid
                        ProcessIdentity      = $committedOwnerIdentity
                        ResumedFromCommitted = $true
                        StatePath            = $statePath
                    }
                    return $resumedCommittedResult
                } else {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                        embedded_owner_identity = $(if ($priorEmbeddedIdentity) { $priorEmbeddedIdentity } else { 'none' })
                        embedded_owner_exited = $priorEmbeddedExited
                        reason = "committed_owner_not_running:$committedOwnerIdentity"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw "Cutover held: committed external owner $committedOwnerIdentity is no longer running (observed=$($mcpObs.State), identity=$($mcpObs.ProcessIdentity))"
                }
            }

            if (-not [string]::IsNullOrWhiteSpace($ExpectedEmbeddedIdentity)) {
                $embeddedTargetIdentity = $ExpectedEmbeddedIdentity
            } elseif ($botObs.State -eq 'running') {
                $embeddedTargetIdentity = $botObs.ProcessIdentity
            } elseif ($null -ne $priorState -and $priorState.PSObject.Properties['mode'] -and
                [string]$priorState.mode -eq 'external' -and (-not [bool]$priorState.committed)) {
                if ([string]::IsNullOrWhiteSpace($priorEmbeddedIdentity)) {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                        embedded_owner_identity = ''; embedded_owner_exited = $false
                        reason = 'embedded_exit_evidence_lost'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw 'Cutover held: interrupted cutover is missing embedded owner exit evidence'
                }
                if ($priorEmbeddedIdentity -ne 'none') {
                    $embeddedTargetIdentity = $priorEmbeddedIdentity
                    $embeddedExited = $priorEmbeddedExited
                } else {
                    $embeddedExited = $true
                }
            } else {
                $embeddedExited = $true
            }

            if (-not [string]::IsNullOrWhiteSpace($embeddedTargetIdentity) -and $embeddedTargetIdentity -notmatch '^(\d+)\|(\d+)$') {
                Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                    schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                    embedded_owner_identity = $embeddedTargetIdentity; embedded_owner_exited = $false
                    reason = 'invalid_expected_embedded_identity'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                })
                throw "Cutover held: invalid ExpectedEmbeddedIdentity '$embeddedTargetIdentity'"
            }

            # Validate healthy .codex_discord_rust.drain.identity (state=open) if present, distinguishing it from prepared/sealed drain operations
            if ([IO.File]::Exists($drainIdentityPath) -or [IO.Directory]::Exists($drainIdentityPath)) {
                $drainIdRec = $null
                try {
                    $drainIdRec = Read-CdrHealthyDrainIdentity -Path $drainIdentityPath
                } catch {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                        embedded_owner_identity = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { $priorEmbeddedIdentity })
                        embedded_owner_exited = $embeddedExited
                        reason = "drain_identity_invalid:$($_.Exception.Message)"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw "Cutover held: .codex_discord_rust.drain.identity is not a healthy open runtime identity ($($_.Exception.Message))"
                }
                if ($null -ne $drainIdRec) {
                    if (-not [string]::IsNullOrWhiteSpace($embeddedTargetIdentity) -and $embeddedTargetIdentity -match '^(\d+)\|\d+$') {
                        $expectedEmbedPid = [int]$Matches[1]
                        if ($drainIdRec.Pid -ne $expectedEmbedPid) {
                            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                                schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                                embedded_owner_identity = $embeddedTargetIdentity; embedded_owner_exited = $embeddedExited
                                reason = "drain_identity_pid_mismatch:$($drainIdRec.Pid)!=$expectedEmbedPid"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                            })
                            throw "Cutover held: .codex_discord_rust.drain.identity pid $($drainIdRec.Pid) does not match embedded owner $embeddedTargetIdentity"
                        }
                    }
                    $embeddedDrainRuntimeId = $drainIdRec.RuntimeId
                }
            }

            $intentLock = Enter-CdrIntentLock -RepoRoot $fullRoot
            try {
                if ($AllowClearManualStop) {
                    Set-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp' -DesiredState 'running' -Action 'mcp-cutover-start' -Source 'mcp-cutover-start' | Out-Null
                }
                $mcpIntentBefore = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp'
                if ($mcpIntentBefore.DesiredState -eq 'stopped' -or (Test-CdrManualStopRequested -RepoRoot $fullRoot -Component 'mcp')) {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                        embedded_owner_identity = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { 'none' })
                        embedded_owner_exited = $embeddedExited
                        reason = 'mcp_manual_stop_active'; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw "Cutover held: MCP desired state is stopped (revision=$($mcpIntentBefore.Revision)) before cutover"
                }
                $authorizedMcpRevision = [uint64]$mcpIntentBefore.Revision
            } finally {
                Exit-CdrIntentLock $intentLock
            }

            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version            = 1
                mode                      = 'external'
                state                     = 'cutover_pending'
                committed                 = $false
                embedded_owner_identity   = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { 'none' })
                embedded_drain_runtime_id = $embeddedDrainRuntimeId
                embedded_owner_exited     = [bool]$embeddedExited
                authorized_mcp_revision   = $authorizedMcpRevision
                updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
        } finally {
            Exit-CdrComponentShortLock $lock
        }
        if ($null -ne $resumedCommittedResult) {
            return $resumedCommittedResult
        }

        # 2. Request binding handoff and prove embedded owner termination before starting external MCP
        $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
        if (-not [string]::IsNullOrWhiteSpace($embeddedTargetIdentity)) {
            $null = $embeddedTargetIdentity -match '^(\d+)\|(\d+)$'
            $embeddedPid = [int]$Matches[1]
            $initialEmbedObs = Get-CdrProcessObservation -ProcessId $embeddedPid -ExpectedIdentity $embeddedTargetIdentity -ExpectedBinaryPath $BotBinaryPath
            if ($initialEmbedObs.State -eq 'stopped') {
                $embeddedExited = $true
            } elseif ($initialEmbedObs.State -eq 'unknown') {
                Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                    schema_version            = 1
                    mode                      = 'external'
                    state                     = 'cutover_failed_hold'
                    committed                 = $false
                    embedded_owner_identity   = $embeddedTargetIdentity
                    embedded_drain_runtime_id = $embeddedDrainRuntimeId
                    embedded_owner_exited     = $false
                    reason                    = "embedded_observation_unknown:$($initialEmbedObs.Reason)"
                    updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                })
                throw "Cutover held: embedded MCP owner $embeddedTargetIdentity observation is unknown ($($initialEmbedObs.Reason))"
            } else {
                $embedDeadline = [DateTime]::UtcNow.AddMilliseconds($EmbeddedExitTimeoutMs)
                $isSyntheticPowershellOwner = (-not [string]::IsNullOrWhiteSpace($initialEmbedObs.BinaryPath)) -and
                    ([IO.Path]::GetFileName($initialEmbedObs.BinaryPath) -ieq 'powershell.exe')

                # Wait briefly for .codex_discord_rust.drain.identity if a real cdr-runtime embedded owner just started
                if ([string]::IsNullOrWhiteSpace($embeddedDrainRuntimeId) -and -not $isSyntheticPowershellOwner) {
                    $drainIdWaitDeadline = [DateTime]::UtcNow.AddMilliseconds([Math]::Min(3000, [Math]::Max(200, $EmbeddedExitTimeoutMs)))
                    while ([string]::IsNullOrWhiteSpace($embeddedDrainRuntimeId) -and [DateTime]::UtcNow -lt $drainIdWaitDeadline) {
                        if ([IO.File]::Exists($drainIdentityPath)) {
                            try {
                                $waitDrainRec = Read-CdrHealthyDrainIdentity -Path $drainIdentityPath
                                if ($null -ne $waitDrainRec -and $waitDrainRec.Pid -eq $embeddedPid) {
                                    $embeddedDrainRuntimeId = $waitDrainRec.RuntimeId
                                    break
                                }
                            } catch { }
                        }
                        $probeNow = Get-CdrProcessObservation -ProcessId $embeddedPid -ExpectedIdentity $embeddedTargetIdentity -ExpectedBinaryPath $BotBinaryPath
                        if ($probeNow.State -ne 'running') { break }
                        Start-Sleep -Milliseconds 30
                    }
                }

                $drainNonce = ''
                $embeddedExited = $false
                try {
                    if (-not [string]::IsNullOrWhiteSpace($embeddedDrainRuntimeId)) {
                        $drainNonce = "cutover-$([Guid]::NewGuid().ToString('N'))"
                        Publish-CdrCutoverDrainFenceMarkers `
                            -RestartMarkerPath $restartMarkerPath `
                            -DrainPreparePath $drainPreparePath `
                            -RuntimeId $embeddedDrainRuntimeId `
                            -ProcessIdentity $embeddedTargetIdentity `
                            -Nonce $drainNonce `
                            -OnBeforeMarkerPublish $OnBeforeDrainMarkerPublish
                    }
                    $parent = Split-Path -Parent $mcpPaths.StopPath
                    if ($parent -and -not [IO.Directory]::Exists($parent)) {
                        [IO.Directory]::CreateDirectory($parent) | Out-Null
                    }
                    if (-not [IO.File]::Exists($mcpPaths.StopPath)) {
                        [IO.File]::WriteAllText($mcpPaths.StopPath, "restart_handoff`n", [Text.UTF8Encoding]::new($false))
                    }

                    while ([DateTime]::UtcNow -lt $embedDeadline) {
                        $eObs = Get-CdrProcessObservation -ProcessId $embeddedPid -ExpectedIdentity $embeddedTargetIdentity -ExpectedBinaryPath $BotBinaryPath
                        if ($eObs.State -eq 'stopped') {
                            $embeddedExited = $true
                            break
                        }
                        if ($eObs.State -eq 'unknown') {
                            break
                        }
                        Start-Sleep -Milliseconds 50
                    }
                } catch {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version            = 1
                        mode                      = 'external'
                        state                     = 'cutover_failed_hold'
                        committed                 = $false
                        embedded_owner_identity   = $embeddedTargetIdentity
                        embedded_drain_runtime_id = $embeddedDrainRuntimeId
                        embedded_owner_exited     = $false
                        reason                    = "embedded_drain_publish_failed:$($_.Exception.Message)"
                        updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw
                } finally {
                    foreach ($transientMarker in @($drainPreparePath, $drainAckPath, $restartMarkerPath)) {
                        if ([IO.File]::Exists($transientMarker)) {
                            try { [IO.File]::Delete($transientMarker) } catch { }
                        }
                    }
                    if ([IO.File]::Exists($mcpPaths.StopPath)) {
                        try {
                            $stopRaw = [IO.File]::ReadAllText($mcpPaths.StopPath, [Text.UTF8Encoding]::UTF8).Trim().ToLowerInvariant()
                            if ($stopRaw -eq 'restart_handoff') {
                                [IO.File]::Delete($mcpPaths.StopPath)
                            }
                        } catch { }
                    }
                }
                if (-not $embeddedExited) {
                    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                        schema_version            = 1
                        mode                      = 'external'
                        state                     = 'cutover_failed_hold'
                        committed                 = $false
                        embedded_owner_identity   = $embeddedTargetIdentity
                        embedded_drain_runtime_id = $embeddedDrainRuntimeId
                        embedded_owner_exited     = $false
                        reason                    = "embedded_owner_did_not_exit:$embeddedTargetIdentity"
                        updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    })
                    throw "Cutover held: embedded MCP owner $embeddedTargetIdentity did not exit within ${EmbeddedExitTimeoutMs}ms"
                }
            }
            if ([IO.File]::Exists($drainIdentityPath)) {
                try {
                    $drainRawText = [IO.File]::ReadAllText($drainIdentityPath, [Text.UTF8Encoding]::UTF8)
                    if ($drainRawText -match "(?m)^pid=$embeddedPid\r?$") {
                        [IO.File]::Delete($drainIdentityPath)
                    }
                } catch { }
            }
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version            = 1
                mode                      = 'external'
                state                     = 'cutover_pending'
                committed                 = $false
                embedded_owner_identity   = $embeddedTargetIdentity
                embedded_drain_runtime_id = $embeddedDrainRuntimeId
                embedded_owner_exited     = $true
                embedded_exited_at_ms     = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                authorized_mcp_revision   = $authorizedMcpRevision
                updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
        }

        # 3. Start external MCP once under the shared journaled launch contract without clearing any Stop issued during wait
        $launchArgs = if ($McpArguments -and $McpArguments.Count -gt 0) {
            $McpArguments
        } else {
            @('--env', (Join-Path $fullRoot '.env'))
        }
        $launch = $null
        $prevResumeEnv = [Environment]::GetEnvironmentVariable('CODEX_REMOTE_MCP_RESTART_RESUME', 'Process')
        try {
            [Environment]::SetEnvironmentVariable('CODEX_REMOTE_MCP_RESTART_RESUME', '1', 'Process')
            $launch = Invoke-CdrJournaledProcessLaunch `
                -RepoRoot $fullRoot `
                -Component 'mcp' `
                -FilePath $resolvedMcpBin `
                -ArgumentList $launchArgs `
                -WorkingDirectory $fullRoot `
                -TargetBinaryPath $resolvedMcpBin `
                -Action 'mcp-cutover-start' `
                -ExpectedIntentRevision $authorizedMcpRevision
            if ($null -eq $launch -or $launch.ProcessId -le 0 -or ($launch.PSObject.Properties['Superseded'] -and $launch.Superseded)) {
                throw 'External MCP launch superseded by concurrent stop intent during cutover'
            }
        } catch {
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version            = 1
                mode                      = 'external'
                state                     = 'cutover_failed_hold'
                committed                 = $false
                embedded_owner_identity   = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { 'none' })
                embedded_drain_runtime_id = $embeddedDrainRuntimeId
                embedded_owner_exited     = [bool]$embeddedExited
                reason                    = "external_launch_failed:$($_.Exception.Message)"
                updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
            throw
        } finally {
            [Environment]::SetEnvironmentVariable('CODEX_REMOTE_MCP_RESTART_RESUME', $prevResumeEnv, 'Process')
        }

        # 4. Verify exact child identity + full strict external status contract (schema, artifact, freshness, config, strict booleans)
        $ready = $false
        $deadline = [DateTime]::UtcNow.AddMilliseconds($ReadinessTimeoutMs)
        $statusCandidates = @($mcpPaths.StatusPath, (Join-Path $fullRoot 'bot_data\remote_mcp.status.json')) | Select-Object -Unique
        $expectedMcpArtifactHash = if ($null -ne $launch.Journal -and $launch.Journal.PSObject.Properties['ArtifactHash']) { [string]$launch.Journal.ArtifactHash } else { '' }
        while ([DateTime]::UtcNow -lt $deadline) {
            if (Test-CdrManualStopRequested -RepoRoot $fullRoot -Component 'mcp') {
                break
            }
            $childObs = Get-CdrProcessObservation -ProcessId $launch.ProcessId -ExpectedIdentity $launch.ProcessIdentity -ExpectedBinaryPath $resolvedMcpBin -Process $launch.Process
            if ($childObs.State -ne 'running') {
                break
            }
            if ([IO.File]::Exists($mcpPaths.InstanceLockPath)) {
                try {
                    $lockRec = Read-CdrMcpInstanceLockRecord -LockPath $mcpPaths.InstanceLockPath
                    if ($null -ne $lockRec -and
                        [int]$lockRec.ProcessId -eq $launch.ProcessId -and
                        [string]$lockRec.ProcessIdentity -ceq $launch.ProcessIdentity) {
                        foreach ($stPath in $statusCandidates) {
                            if (Test-CdrExternalMcpCutoverStatusReady `
                                -StatusPath $stPath `
                                -McpPaths $mcpPaths `
                                -ExpectedPid ([int]$launch.ProcessId) `
                                -ExpectedIdentity ([string]$launch.ProcessIdentity) `
                                -ExpectedBinaryPath $resolvedMcpBin `
                                -ExpectedArtifactHash $expectedMcpArtifactHash) {
                                $ready = $true
                                break
                            }
                        }
                    }
                    if ($ready) { break }
                } catch { }
            }
            Start-Sleep -Milliseconds 50
        }

        # 5. Hold on failure (NEVER automatically roll back to embedded mode)
        if (-not $ready) {
            $stopAbort = Stop-CdrComponentRuntime -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $resolvedMcpBin -GraceTimeoutMs 1000 -Source 'cutover-failed-hold' -PreserveIntent -AllowForceKill
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version            = 1
                mode                      = 'external'
                state                     = 'cutover_failed_hold'
                committed                 = $false
                embedded_owner_identity   = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { 'none' })
                embedded_drain_runtime_id = $embeddedDrainRuntimeId
                embedded_owner_exited     = [bool]$embeddedExited
                external_stopped          = [bool]$stopAbort.Stopped
                updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
            throw 'External MCP cutover failed readiness verification; held in cutover_failed_hold (no automatic rollback to embedded)'
        }

        # 6. Prove no supported legacy helper or unexpected embedded bot became active before lifting fences and committing external ownership
        $postHelperCheck = $null
        try {
            $postHelperCheck = Wait-CdrLegacyHelpersExited -RepoRoot $fullRoot -TimeoutMs $LegacyHelperExitTimeoutMs
        } catch {
            $stopAbort = Stop-CdrComponentRuntime -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $resolvedMcpBin -GraceTimeoutMs 1000 -Source 'cutover-failed-hold' -PreserveIntent -AllowForceKill
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version            = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                embedded_owner_identity   = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { 'none' })
                embedded_drain_runtime_id = $embeddedDrainRuntimeId
                embedded_owner_exited     = [bool]$embeddedExited
                external_stopped          = [bool]$stopAbort.Stopped
                reason                    = "legacy_helper_observation_failed:$($_.Exception.Message)"
                updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
            throw "Cutover held before lifting fence: legacy helper observation failed ($($_.Exception.Message))"
        }
        if (-not $postHelperCheck.AllExited) {
            $firstHelper = $postHelperCheck.Active[0]
            $stopAbort = Stop-CdrComponentRuntime -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $resolvedMcpBin -GraceTimeoutMs 1000 -Source 'cutover-failed-hold' -PreserveIntent -AllowForceKill
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version            = 1; mode = 'external'; state = 'cutover_failed_hold'; committed = $false
                embedded_owner_identity   = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { 'none' })
                embedded_drain_runtime_id = $embeddedDrainRuntimeId
                embedded_owner_exited     = [bool]$embeddedExited
                external_stopped          = [bool]$stopAbort.Stopped
                reason                    = "legacy_helper_still_active:$($firstHelper.HelperName):$($firstHelper.ProcessId)"
                updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
            throw "Cutover held before lifting fence: legacy helper $($firstHelper.HelperName) (PID $($firstHelper.ProcessId)) is still active"
        }

        # 7. Atomically commit external ownership BEFORE removing cutover/disabled fences (R4)
        $commitShortLock = Enter-CdrComponentShortLock -RepoRoot $fullRoot -Component 'common'
        try {
            $commitIntentLock = Enter-CdrIntentLock -RepoRoot $fullRoot
            try {
                $mcpIntentAtCommit = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp'
                if ($mcpIntentAtCommit.DesiredState -eq 'stopped' -or
                    [uint64]$mcpIntentAtCommit.Revision -ne $authorizedMcpRevision -or
                    (Test-CdrManualStopRequested -RepoRoot $fullRoot -Component 'mcp')) {
                    throw 'External MCP intent was superseded or stopped before final ownership commit'
                }
                $childAtCommit = Get-CdrProcessObservation -ProcessId $launch.ProcessId -ExpectedIdentity $launch.ProcessIdentity -ExpectedBinaryPath $resolvedMcpBin -Process $launch.Process
                if ($childAtCommit.State -ne 'running') {
                    throw "External MCP process $($launch.ProcessIdentity) exited before final ownership commit"
                }
                if ($null -ne $OnBeforeCommitWrite) {
                    & $OnBeforeCommitWrite $statePath
                }
                Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                    schema_version            = 1
                    mode                      = 'external'
                    state                     = 'committed'
                    committed                 = $true
                    owner_pid                 = [int]$launch.ProcessId
                    owner_identity            = [string]$launch.ProcessIdentity
                    embedded_owner_identity   = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { 'none' })
                    embedded_drain_runtime_id = $embeddedDrainRuntimeId
                    embedded_owner_exited     = [bool]$embeddedExited
                    updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                })
                $verifiedCommit = [IO.File]::ReadAllText($statePath, [Text.UTF8Encoding]::UTF8) | ConvertFrom-Json -ErrorAction Stop
                if ($null -eq $verifiedCommit -or
                    [int]$verifiedCommit.schema_version -ne 1 -or
                    [string]$verifiedCommit.mode -cne 'external' -or
                    [string]$verifiedCommit.state -cne 'committed' -or
                    $verifiedCommit.committed -isnot [bool] -or
                    (-not $verifiedCommit.committed) -or
                    [string]$verifiedCommit.owner_identity -cne [string]$launch.ProcessIdentity) {
                    throw 'Ownership state verification failed after atomic commit write'
                }
            } finally {
                Exit-CdrIntentLock $commitIntentLock
            }
        } catch {
            $commitErr = $_
            $stopAbort = Stop-CdrComponentRuntime -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $resolvedMcpBin -GraceTimeoutMs 1000 -Source 'cutover-failed-hold' -PreserveIntent -AllowForceKill
            try {
                Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                    schema_version            = 1
                    mode                      = 'external'
                    state                     = 'cutover_failed_hold'
                    committed                 = $false
                    embedded_owner_identity   = $(if ($embeddedTargetIdentity) { $embeddedTargetIdentity } else { 'none' })
                    embedded_drain_runtime_id = $embeddedDrainRuntimeId
                    embedded_owner_exited     = [bool]$embeddedExited
                    external_stopped          = [bool]$stopAbort.Stopped
                    reason                    = "commit_write_failed:$($commitErr.Exception.Message)"
                    updated_at_ms             = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                })
            } catch { }
            throw "Cutover held before lifting fence: final ownership commit failed ($($commitErr.Exception.Message))"
        } finally {
            Exit-CdrComponentShortLock $commitShortLock
        }

        if ($null -ne $OnAfterCommitBeforeFenceRelease) {
            & $OnAfterCommitBeforeFenceRelease $statePath
        }

        if ([IO.File]::Exists($cutoverMarkerPath)) {
            [IO.File]::Delete($cutoverMarkerPath)
        }
        if ([IO.File]::Exists($disabledFencePath) -and ([IO.File]::ReadAllText($disabledFencePath).Trim() -ceq 'mcp_cutover_fence')) {
            [IO.File]::Delete($disabledFencePath)
        }

        return [pscustomobject]@{
            Committed       = $true
            Mode            = 'external'
            Process         = $launch.Process
            ProcessId       = [int]$launch.ProcessId
            ProcessIdentity = [string]$launch.ProcessIdentity
            StatePath       = $statePath
        }
    } finally {
        if ($null -ne $legacyControlHandle) {
            $legacyControlHandle.Dispose()
        }
    }
}

function Invoke-CdrMcpRollbackToEmbedded {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [string]$McpBinaryPath = '',
        [switch]$StopExternalFirst,
        [switch]$AllowForceKillExternal,
        [int]$GraceTimeoutMs = 45000
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $statePath = Join-Path $fullRoot '.cdr-maintenance\mcp-ownership-state.json'

    $initialObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath
    if ($initialObs.State -eq 'unknown') {
        throw "Rollback to embedded refused: external MCP observation is unknown ($($initialObs.Reason))"
    }
    $journal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component 'mcp'
    if ($null -ne $journal -and $journal.Phase -in @('launching', 'launch_outcome_unknown')) {
        throw "Rollback to embedded refused: external MCP launch journal is in $($journal.Phase) phase"
    }
    if ($initialObs.State -eq 'running' -and -not $StopExternalFirst) {
        throw "Rollback to embedded refused: external MCP process $($initialObs.ProcessIdentity) is still running. Pass -StopExternalFirst to stop external MCP before switching mode."
    }

    # Fence external MCP start authority FIRST under the shared control lock before proving termination
    Set-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp' -DesiredState 'stopped' -Action 'rollback-to-embedded' -Source 'rollback-to-embedded' | Out-Null

    $postFenceObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath
    if ($postFenceObs.State -eq 'unknown') {
        throw "Rollback to embedded refused: external MCP observation is unknown after fencing ($($postFenceObs.Reason))"
    }
    if ($postFenceObs.State -eq 'running') {
        if (-not $StopExternalFirst) {
            throw "Rollback to embedded refused: external MCP process $($postFenceObs.ProcessIdentity) is running."
        }
        $stopRes = Stop-CdrComponentRuntime -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath `
            -GraceTimeoutMs $GraceTimeoutMs -Source 'rollback-to-embedded' -PreserveIntent -AllowForceKill:$AllowForceKillExternal
        if (-not $stopRes.Stopped) {
            Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
                schema_version = 1; mode = 'external'; state = 'rollback_failed_hold'; committed = $false
                reason = "external_mcp_not_stopped:$($stopRes.State)"; updated_at_ms = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            })
            throw "Rollback to embedded refused: external MCP process $($postFenceObs.ProcessIdentity) did not terminate cleanly (state=$($stopRes.State))."
        }
    }

    $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
    foreach ($p in @($mcpPaths.StatusPath, (Join-Path $fullRoot 'bot_data\remote_mcp.status.json'), $mcpPaths.InstanceLockPath, (Join-Path $fullRoot '.codex_discord_runtime.cutover'))) {
        if ([IO.File]::Exists($p)) { [IO.File]::Delete($p) }
    }
    $disabledFencePath = Join-Path $fullRoot '.codex_discord_bot.disabled'
    if ([IO.File]::Exists($disabledFencePath) -and ([IO.File]::ReadAllText($disabledFencePath).Trim() -ceq 'mcp_cutover_fence')) {
        [IO.File]::Delete($disabledFencePath)
    }

    Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{
        schema_version = 1
        mode           = 'embedded'
        state          = 'committed'
        committed      = $true
        updated_at_ms  = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    })

    return [pscustomobject]@{
        RolledBack = $true
        Mode       = 'embedded'
        StatePath  = $statePath
    }
}

function Invoke-CdrMcpWatchdog {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [string]$McpBinaryPath = '',
        [switch]$DryRun
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
    if (-not $mcpPaths.Enabled -or $mcpPaths.EffectiveOwnership -ne 'external' -or -not $mcpPaths.OwnershipCommitted) {
        return [pscustomobject]@{ State = 'not_external'; ProcessId = 0; ProcessIdentity = '' }
    }
    if (Test-CdrManualStopRequested -RepoRoot $fullRoot -Component 'mcp') {
        return [pscustomobject]@{ State = 'stopped_by_intent'; ProcessId = 0; ProcessIdentity = '' }
    }
    $obs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath
    if ($obs.State -eq 'running') {
        return [pscustomobject]@{ State = 'running'; ProcessId = $obs.ProcessId; ProcessIdentity = $obs.ProcessIdentity }
    }
    if ($obs.State -eq 'unknown') {
        return [pscustomobject]@{ State = 'unknown_hold'; ProcessId = $obs.ProcessId; ProcessIdentity = $obs.ProcessIdentity; Reason = $obs.Reason }
    }
    $journal = Read-CdrComponentLaunchJournal -RepoRoot $fullRoot -Component 'mcp'
    if ($null -ne $journal -and $journal.Phase -in @('launching', 'launch_outcome_unknown')) {
        return [pscustomobject]@{ State = 'unknown_hold'; ProcessId = 0; ProcessIdentity = ''; Reason = $journal.Phase }
    }
    if ($DryRun) {
        return [pscustomobject]@{ State = 'would_start'; ProcessId = 0; ProcessIdentity = '' }
    }
    $cands = Resolve-CdrComponentBinaryCandidates -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath
    if ($cands.Count -eq 0) {
        return [pscustomobject]@{ State = 'binary_missing'; ProcessId = 0; ProcessIdentity = '' }
    }
    $resolvedBin = $cands[0]
    $stagedMcpPath = Join-Path $fullRoot '.cdr-maintenance\staged-cdr-remote-agent.exe'
    if ([IO.File]::Exists($stagedMcpPath)) {
        Copy-CdrAtomicBinary -SourcePath $stagedMcpPath -DestinationPath $resolvedBin
        [IO.File]::Delete($stagedMcpPath)
    }
    $intent = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp'
    try {
        $launch = Invoke-CdrJournaledProcessLaunch `
            -RepoRoot $fullRoot `
            -Component 'mcp' `
            -FilePath $resolvedBin `
            -ArgumentList @('--env', (Join-Path $fullRoot '.env')) `
            -WorkingDirectory $fullRoot `
            -TargetBinaryPath $resolvedBin `
            -Action 'mcp-watchdog-start' `
            -ExpectedIntentRevision $intent.Revision
        return [pscustomobject]@{
            State           = 'started'
            ProcessId       = [int]$launch.ProcessId
            ProcessIdentity = [string]$launch.ProcessIdentity
        }
    } catch {
        if ($_.Exception.Message -like 'cdr_already_running*') {
            $afterObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath
            if ($afterObs.State -eq 'running') {
                return [pscustomobject]@{ State = 'running'; ProcessId = [int]$afterObs.ProcessId; ProcessIdentity = [string]$afterObs.ProcessIdentity }
            }
        }
        throw
    }
}

function Invoke-CdrAppLaunch {
    param(
        [Parameter(Mandatory)][Alias('Root')][string]$RepoRoot,
        [string]$BotBinaryPath = '',
        [string]$McpBinaryPath = '',
        [string[]]$BotArguments = @(),
        [string[]]$McpArguments = @(),
        [switch]$SkipTray
    )
    $fullRoot = [IO.Path]::GetFullPath($RepoRoot)
    $trayRuntimeScript = Join-Path $fullRoot 'codex-discord-tray-runtime.ps1'
    if (-not [IO.File]::Exists($trayRuntimeScript)) {
        $trayRuntimeScript = Join-Path (Split-Path -Parent $PSScriptRoot) 'codex-discord-tray-runtime.ps1'
    }
    if ([IO.File]::Exists($trayRuntimeScript) -and -not (Get-Command Get-CdrTrayMutexName -ErrorAction SilentlyContinue)) {
        $ScriptDir = $fullRoot
        . $trayRuntimeScript
    }

    $botObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'bot' -BinaryPath $BotBinaryPath
    $mcpObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath
    $trayRunning = if (Get-Command Get-CdrTrayMutexBusy -ErrorAction SilentlyContinue) {
        Get-CdrTrayMutexBusy (Get-CdrTrayMutexName $fullRoot)
    } else {
        $false
    }

    $botIntent = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'bot'
    $mcpIntent = Get-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp'

    $appAlreadyActive = ($botObs.State -eq 'running') -or ($mcpObs.State -eq 'running') -or $trayRunning
    if (-not $appAlreadyActive) {
        # Explicit initial app launch or explicit app launch after '전체 종료' (exit-all):
        # Clear 'exit-all' stop intents so both configured components start fresh.
        # However, if bot was individually stopped via manual 'stop' / 'tray-stop-bot', preserve that manual stop!
        if ($botIntent.DesiredState -eq 'stopped' -and $botIntent.Action -eq 'exit-all') {
            $botIntent = Set-CdrComponentIntent -RepoRoot $fullRoot -Component 'bot' -DesiredState 'running' -Action 'app-launch' -Source 'app-launcher'
        }
        if ($mcpIntent.DesiredState -eq 'stopped' -and $mcpIntent.Action -eq 'exit-all') {
            $mcpIntent = Set-CdrComponentIntent -RepoRoot $fullRoot -Component 'mcp' -DesiredState 'running' -Action 'app-launch' -Source 'app-launcher'
        }
    }

    # 1. Start or verify External MCP independently of Bot startup
    $mcpState = 'not_configured'
    $mcpPid = 0
    $mcpIdentity = ''
    $mcpPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $fullRoot
    if ($mcpPaths.Enabled -and $mcpPaths.EffectiveOwnership -eq 'external' -and $mcpPaths.OwnershipCommitted) {
        if ($mcpObs.State -eq 'running') {
            $mcpState = 'already_running'
            $mcpPid = [int]$mcpObs.ProcessId
            $mcpIdentity = [string]$mcpObs.ProcessIdentity
        } elseif ($mcpObs.State -eq 'unknown') {
            $mcpState = 'unknown_hold'
        } elseif ($mcpIntent.DesiredState -eq 'stopped' -or (Test-CdrManualStopRequested -RepoRoot $fullRoot -Component 'mcp')) {
            $mcpState = 'stopped_by_intent'
        } else {
            $mcpCands = Resolve-CdrComponentBinaryCandidates -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath
            if ($mcpCands.Count -gt 0) {
                $mArgs = if ($McpArguments -and $McpArguments.Count -gt 0) { $McpArguments } else { @('--env', (Join-Path $fullRoot '.env')) }
                try {
                    $mLaunch = Invoke-CdrJournaledProcessLaunch `
                        -RepoRoot $fullRoot -Component 'mcp' -FilePath $mcpCands[0] `
                        -ArgumentList $mArgs -WorkingDirectory $fullRoot -TargetBinaryPath $mcpCands[0] `
                        -Action 'app-launch-mcp' -ExpectedIntentRevision $mcpIntent.Revision
                    $mcpState = 'started'
                    $mcpPid = [int]$mLaunch.ProcessId
                    $mcpIdentity = [string]$mLaunch.ProcessIdentity
                } catch {
                    if ($_.Exception.Message -like 'cdr_already_running*') {
                        $afterMcpObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'mcp' -BinaryPath $McpBinaryPath
                        $mcpState = 'already_running'
                        $mcpPid = [int]$afterMcpObs.ProcessId
                        $mcpIdentity = [string]$afterMcpObs.ProcessIdentity
                    } else {
                        $mcpState = "launch_failed:$($_.Exception.Message)"
                    }
                }
            } else {
                $mcpState = 'binary_missing'
            }
        }
    }

    # 2. Start or verify Bot independently (a bot startup failure never aborts MCP or Tray!)
    $botState = 'stopped_by_intent'
    $botPid = 0
    $botIdentity = ''
    $botError = ''
    if ($botObs.State -eq 'running') {
        $botState = 'already_running'
        $botPid = [int]$botObs.ProcessId
        $botIdentity = [string]$botObs.ProcessIdentity
    } elseif ($botObs.State -eq 'unknown') {
        $botState = 'unknown_hold'
    } elseif ($botIntent.DesiredState -eq 'stopped' -or (Test-CdrManualStopRequested -RepoRoot $fullRoot -Component 'bot')) {
        $botState = 'stopped_by_intent'
    } else {
        $botCands = Resolve-CdrComponentBinaryCandidates -RepoRoot $fullRoot -Component 'bot' -BinaryPath $BotBinaryPath
        if ($botCands.Count -gt 0) {
            $bArgs = if ($BotArguments -and $BotArguments.Count -gt 0) { $BotArguments } else { @('--env', (Join-Path $fullRoot '.env')) }
            $botOutLog = Join-Path $fullRoot 'codex_discord_rust.log'
            $botErrLog = Join-Path $fullRoot 'codex_discord_rust.error.log'
            try {
                $bLaunch = Invoke-CdrJournaledProcessLaunch `
                    -RepoRoot $fullRoot -Component 'bot' -FilePath $botCands[0] `
                    -ArgumentList $bArgs -WorkingDirectory $fullRoot -TargetBinaryPath $botCands[0] `
                    -Action 'app-launch-bot' -ExpectedIntentRevision $botIntent.Revision `
                    -RedirectStandardOutput $botOutLog -RedirectStandardError $botErrLog
                if ($null -ne $bLaunch.Journal -and $bLaunch.Journal.RequireStartupRecord -and
                    (Get-Command Assert-CdrStartedArtifact -ErrorAction SilentlyContinue)) {
                    $botLockPath = Join-Path $fullRoot '.codex_discord_rust.runtime.lock'
                    $startupDeadline = [DateTime]::UtcNow.AddSeconds(2)
                    while ([DateTime]::UtcNow -lt $startupDeadline) {
                        if ($null -ne $bLaunch.Process -and $bLaunch.Process.PSObject.Properties['HasExited'] -and $bLaunch.Process.HasExited) {
                            break
                        }
                        if ([IO.File]::Exists($botLockPath)) {
                            $lockTxt = [IO.File]::ReadAllText($botLockPath, [Text.UTF8Encoding]::UTF8)
                            if ($lockTxt -match "(?m)^pid=$($bLaunch.ProcessId)\r?$") {
                                Assert-CdrStartedArtifact -Root $fullRoot -BinaryPath $botCands[0] `
                                    -ExpectedIdentity $bLaunch.ProcessIdentity -Artifact $bLaunch.Journal
                                break
                            }
                        }
                        Start-Sleep -Milliseconds 50
                    }
                }
                $botState = 'started'
                $botPid = [int]$bLaunch.ProcessId
                $botIdentity = [string]$bLaunch.ProcessIdentity
            } catch {
                if ($_.Exception.Message -like 'cdr_already_running*') {
                    $afterBotObs = Get-CdrComponentObservation -RepoRoot $fullRoot -Component 'bot' -BinaryPath $BotBinaryPath
                    $botState = 'already_running'
                    $botPid = [int]$afterBotObs.ProcessId
                    $botIdentity = [string]$afterBotObs.ProcessIdentity
                } else {
                    $botState = 'launch_failed'
                    $botError = $_.Exception.Message
                }
            }
        } else {
            $botState = 'binary_missing'
        }
    }

    # 3. Ensure single Tray instance for this install root
    $trayResult = [pscustomobject]@{ State = 'skipped'; Pid = $null }
    if (-not $SkipTray -and (Get-Command Start-CdrTrayStandAlone -ErrorAction SilentlyContinue)) {
        $trayResult = Start-CdrTrayStandAlone -Root $fullRoot
    }

    return [pscustomobject]@{
        Reopened       = $appAlreadyActive
        BotState       = $botState
        BotProcessId   = $botPid
        BotIdentity    = $botIdentity
        BotError       = $botError
        McpState       = $mcpState
        McpProcessId   = $mcpPid
        McpIdentity    = $mcpIdentity
        TrayState      = [string]$trayResult.State
        TrayProcessId  = $trayResult.Pid
    }
}
