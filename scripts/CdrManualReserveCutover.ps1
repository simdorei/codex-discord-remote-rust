# One forced cutover; .disabled is the persistent seal. No .stop publication.
# The entry point loads the pinned watchdog/control helpers before this module.
function Save-CdrCutoverPhase {
    param([string]$Phase)
    $script:CutoverState.Phase=$Phase
    $script:CutoverState.UpdatedAt=[DateTimeOffset]::UtcNow.ToString('o')
    Write-AtomicRestartMarker $script:CutoverStatePath ($script:CutoverState | ConvertTo-Json -Depth 12 -Compress)
}

function Test-CdrCutoverSettled {
    param($State)
    # A persisted completion still needs monitored cleanup while any seal exists.
    # The worker, never the monitor, verifies ownership before removing that seal.
    return $State.Phase -eq 'complete' -and -not [IO.File]::Exists($DisablePath)
}

function Assert-CdrCutoverSeal {
    if (-not [IO.File]::Exists($DisablePath) -or [IO.File]::ReadAllText($DisablePath) -cne $script:CutoverSeal) {
        throw 'Cutover disable seal is missing or belongs to another owner.'
    }
    if ([IO.File]::Exists($StopPath)) { throw 'Foreign stop intent preserved; cutover cannot launch.' }
    Assert-CdrNoPendingRestart $RepoRoot
}

function Get-CdrPinnedProcess {
    param($Pin)
    if ($Pin.Identity -notmatch '^(\d+)\|\d+$') { throw 'Invalid captured process identity.' }
    $process=Get-Process -Id ([int]$Matches[1]) -ErrorAction SilentlyContinue
    if ($null -eq $process) { return $null }
    try {
        [void]$process.Handle
        if ($process.HasExited) { $process.Dispose(); return $null }
        if ((Get-RustProcessIdentity $process) -cne $Pin.Identity -or $process.Path -ine $Pin.Path) {
            throw 'Captured PID was reused or its image changed; foreign process preserved.'
        }
        return $process
    } catch {
        $exited=$process.HasExited
        $process.Dispose()
        if ($exited) { return $null }
        throw
    }
}

function Stop-CdrCapturedProcesses {
    param($Pins)
    # Reentry never reconstructs ancestry after the original parent has disappeared.
    $handles=[Collections.Generic.List[object]]::new()
    try {
        foreach ($pin in $Pins) {
            $process=Get-CdrPinnedProcess $pin
            if ($null -ne $process) {
                if ($process.Id -eq $PID) { throw 'Cutover is inside the captured tree.' }
                $handles.Add($process)
            }
        }
        foreach ($process in $handles) {
            if (-not $process.HasExited) {
                try { $process.Kill() } catch { if (-not $process.HasExited) { throw } }
            }
        }
        foreach ($process in $handles) {
            if (-not $process.WaitForExit(5000)) { throw 'Captured writer did not exit.' }
        }
    } finally { foreach ($process in $handles) { $process.Dispose() } }
}

function Assert-CdrCutoverStopped {
    foreach ($pin in $script:CutoverState.Captured) {
        $process=Get-CdrPinnedProcess $pin
        if ($null -ne $process) { $process.Dispose(); throw 'Captured process is still alive.' }
    }
    # A second same-install binary is neither a safe writer nor a launch candidate.
    foreach ($process in @(Get-Process -Name 'cdr-runtime' -ErrorAction SilentlyContinue)) {
        try { if ($process.Path -ieq $BinaryPath) { throw 'Another installed runtime is alive; store remains sealed.' } }
        finally { $process.Dispose() }
    }
}

function Backup-CdrCutoverStore {
    Assert-CdrCutoverStopped
    $output=@(& $BinaryPath --admin backup-store --repo-root $RepoRoot 2>&1)
    if ($LASTEXITCODE -ne 0) { throw 'Authoritative post-stop SQLite backup failed.' }
    $line=@($output | ForEach-Object { $_.ToString() } | Where-Object { $_ -match '^backup_created path=' })
    if ($line.Count -ne 1) { throw 'Post-stop backup result is not unambiguous.' }
    $path=$line[0].Substring('backup_created path='.Length)
    if (-not [IO.File]::Exists($path)) { throw 'Post-stop SQLite backup is missing.' }
    $script:CutoverState.DatabaseBackup=$path
    $script:CutoverState.DatabaseBackupHash=Get-CdrArtifactHash $path
    $prior=Join-Path $script:CutoverBundle 'previous-runtime.exe'
    if (-not [IO.File]::Exists($prior)) { [IO.File]::Copy($BinaryPath,$prior,$false) }
    if ((Get-CdrArtifactHash $prior) -cne $script:CutoverState.BaselineHash) { throw 'Baseline backup hash mismatch.' }
}

function Install-CdrCutoverCandidate {
    Assert-CdrCutoverStopped
    if ((Get-CdrArtifactHash $BinaryPath) -ceq $script:CutoverState.CandidateHash) { return }
    if ((Get-CdrArtifactHash $BinaryPath) -cne $script:CutoverState.BaselineHash) { throw 'Installed binary changed unexpectedly.' }
    if ((Get-CdrArtifactHash $CandidatePath) -cne $script:CutoverState.CandidateHash) { throw 'Candidate hash changed.' }
    $next=Join-Path (Split-Path -Parent $BinaryPath) ('cdr-runtime.next.'+$script:CutoverState.Operation+'.exe')
    if (-not [IO.File]::Exists($next)) { [IO.File]::Copy($CandidatePath,$next,$false) }
    if ((Get-CdrArtifactHash $next) -cne $script:CutoverState.CandidateHash) { throw 'Staged candidate differs.' }
    [IO.File]::Replace($next,$BinaryPath,(Join-Path $script:CutoverBundle 'replaced-runtime.exe'))
    if ((Get-CdrArtifactHash $BinaryPath) -cne $script:CutoverState.CandidateHash) { throw 'Installed candidate verification failed.' }
}

function Migrate-CdrCutoverStore {
    Assert-CdrCutoverStopped
    $output=@(& $BinaryPath --admin retire-automatic-reserve --repo-root $RepoRoot 2>&1)
    if ($LASTEXITCODE -ne 0) { throw 'Reserve retirement failed; roll-forward only, store remains sealed.' }
    $script:CutoverState.Retirement=($output | ForEach-Object { $_.ToString() }) -join "`n"
}

function Start-CdrCutoverCandidate {
    Assert-CdrCutoverSeal
    if ((Get-CdrArtifactHash $BinaryPath) -cne $script:CutoverState.CandidateHash) { throw 'Candidate changed before launch.' }
    $journalPath=Join-Path $script:CutoverBundle 'launch.json'
    $journal=New-CdrLaunchJournal -Path $journalPath -Operation ('manual-reserve:'+$script:CutoverState.Operation) -ArtifactHash $script:CutoverState.CandidateHash
    if ($journal.Phase -eq 'prepared') {
        Assert-CdrCutoverStopped
        $script:CdrLaunchJournalPath=$journalPath
        try { Start-RustRuntime -ResumeRemoteMcp }
        finally { $script:CdrLaunchJournalPath=$null }
        $journal=Read-CdrLaunchJournal $journalPath
    }
    # 'launching' is deliberately not retried; an unrecorded child may have written.
    $child=Get-CdrRecordedChild $journal
    if ($null -eq $child) { throw 'Recorded candidate exited or launch is uncertain; no second launch.' }
    try { $script:CutoverState.CandidateIdentity=Get-RustProcessIdentity $child }
    finally { $child.Dispose() }
}

function Wait-CdrCutoverReady {
    $first=$null
    $deadline=[DateTimeOffset]::UtcNow.AddSeconds(150)
    while ([DateTimeOffset]::UtcNow -lt $deadline) {
        Assert-CdrCutoverSeal
        if ((Get-VerifiedRuntimeIdentity) -cne $script:CutoverState.CandidateIdentity) { throw 'Candidate runtime ownership changed.' }
        $process=Get-VerifiedRuntimeProcess
        try { $health=Get-HeartbeatHealth $process } finally { $process.Dispose() }
        if ($health.Healthy -and -not $health.Bootstrap) {
            $heartbeat=[IO.File]::ReadAllText($HeartbeatPath)
            if ($heartbeat -match '(?m)^updated_at=(\d+)$') {
                $stamp=[long]$Matches[1]
                if ($null -eq $first) { $first=$stamp }
                $ready=@(Get-Content -LiteralPath $StderrLog -Tail 300 -ErrorAction SilentlyContinue | Where-Object { $_ -match '^discord_ready user=\d+ application=\d+$' }).Count -gt 0
                if ($stamp -gt $first -and $ready) {
                    $script:CutoverState.Readiness=@{ FirstHeartbeat=$first;SecondHeartbeat=$stamp;DiscordReady=$true }
                    return
                }
            }
        }
        Start-Sleep -Milliseconds 500
    }
    throw 'Candidate readiness is unverified; disable seal retained.'
}

function Invoke-CdrManualReserveCutover {
    if ($script:CutoverState.Phase -eq 'prepared') {
        Assert-CdrNoPendingRestart $RepoRoot
        if ([IO.File]::Exists($StopPath)) { throw 'Foreign stop intent preserved.' }
        if ((Get-CdrArtifactHash $BinaryPath) -cne $script:CutoverState.BaselineHash) { throw 'Baseline binary changed before cutover.' }
        if ((Get-VerifiedRuntimeIdentity) -cne $script:CutoverState.TargetIdentity) { throw 'Original runtime identity changed.' }
        if ([IO.File]::Exists($DisablePath)) { Assert-CdrMarkerOwner $DisablePath $script:CutoverSeal }
        else { Write-NewCdrMarker $DisablePath $script:CutoverSeal }
        Save-CdrCutoverPhase 'sealed'
    }
    if ($script:CutoverState.Phase -eq 'complete') {
        if ([IO.File]::Exists($DisablePath)) {
            Assert-CdrCutoverSeal
            if ((Get-VerifiedRuntimeIdentity) -cne $script:CutoverState.CandidateIdentity) { throw 'Completed candidate identity changed; seal retained.' }
            if ((Get-CdrArtifactHash $BinaryPath) -cne $script:CutoverState.CandidateHash) { throw 'Completed candidate binary changed; seal retained.' }
            [IO.File]::Delete($DisablePath)
        }
        return
    }
    Assert-CdrCutoverSeal
    if ($script:CutoverState.Phase -eq 'sealed') {
        if ((Get-CdrArtifactHash $BinaryPath) -cne $script:CutoverState.BaselineHash) { throw 'Baseline binary changed before capture.' }
        $process=Get-VerifiedRuntimeProcess
        if ((Get-RustProcessIdentity $process) -cne $script:CutoverState.TargetIdentity) { throw 'Runtime changed before capture.' }
        $handles=Get-CdrOwnedProcessHandles $process
        try {
            $script:CutoverState.Captured=@($handles | ForEach-Object { @{Identity=(Get-RustProcessIdentity $_);Path=$_.Path} })
            Save-CdrCutoverPhase 'stopping' # Every original handle is still pinned here.
            Stop-CdrCapturedProcesses $script:CutoverState.Captured
        } finally { foreach ($item in $handles) { $item.Dispose() } }
    }
    if ($script:CutoverState.Phase -eq 'stopping') {
        Stop-CdrCapturedProcesses $script:CutoverState.Captured
        Assert-CdrCutoverStopped
        Save-CdrCutoverPhase 'stopped'
    }
    if ($script:CutoverState.Phase -eq 'stopped') {
        Backup-CdrCutoverStore
        Save-CdrCutoverPhase 'backed_up'
    }
    if ($script:CutoverState.Phase -eq 'backed_up') {
        Install-CdrCutoverCandidate
        Save-CdrCutoverPhase 'installed'
    }
    if ($script:CutoverState.Phase -in @('installed','migrating')) {
        Save-CdrCutoverPhase 'migrating' # No DB rewind or auto-enabled baseline launch beyond here.
        Migrate-CdrCutoverStore
        Save-CdrCutoverPhase 'migrated'
    }
    if ($script:CutoverState.Phase -eq 'migrated') { Save-CdrCutoverPhase 'launch_ready' }
    if ($script:CutoverState.Phase -in @('launch_ready','write_possible')) {
        Assert-CdrCutoverSeal
        Save-CdrCutoverPhase 'write_possible'
        Start-CdrCutoverCandidate
        Save-CdrCutoverPhase 'launched'
    }
    if ($script:CutoverState.Phase -eq 'launched') {
        Wait-CdrCutoverReady
        if ((Get-CdrArtifactHash $BinaryPath) -cne $script:CutoverState.CandidateHash) { throw 'Installed binary changed during readiness.' }
        Save-CdrCutoverPhase 'complete'
        Assert-CdrCutoverSeal
        [IO.File]::Delete($DisablePath)
    }
    if ($script:CutoverState.Phase -ne 'complete') { throw 'Unknown cutover phase preserved.' }
}
