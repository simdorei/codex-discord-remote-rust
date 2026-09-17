. (Join-Path $PSScriptRoot 'CdrMaintenanceCompletion.ps1')
function Invoke-CdrMaintenanceLaunch($State, [string]$StatePath) {
    $journalPath = Join-Path $State.Bundle 'launch.json'
    $journal = Read-CdrLaunchJournal $journalPath
    if ($journal) {
        if ($journal.Operation -cne ('maintenance:'+$State.Operation) -or
            $journal.ArtifactHash -cne $State.CandidateHash -or
            -not (Test-RestartDrainFenceMatch $journal.Fence $State.Fence)) { throw 'maintenance_foreign_launch' }
        $child = Get-CdrRecordedChild $journal
        if ($journal.Phase -eq 'child') {
            if (-not $child) { throw 'maintenance_recorded_child_dead; no automatic relaunch' }
            return
        }
    }
    Assert-CdrMaintenanceNoRuntime $State
    Assert-CdrMaintenanceArtifacts $State
    Invoke-CdrMaintenanceFullReadiness $State
    Assert-CdrMaintenanceDeadline $State
    Assert-CdrMaintenanceMarkers $State
    # Only launch_ready authorizes this removal; repeat tolerates its own absent files.
    foreach ($path in @($DrainPreparePath,$DrainAckPath)) {
        if ([IO.File]::Exists($path)) {
            if (-not (Test-RestartDrainFenceMatch (Get-RestartDrainFence $path) $State.Fence)) {
                throw 'maintenance_foreign_launch_fence'
            }
            [IO.File]::Delete($path)
        }
    }
    if ([IO.File]::Exists($DrainIdentityPath)) {
        $fields = Read-RestartDrainFields $DrainIdentityPath @('version','runtime_id','pid','state')
        if ($fields.runtime_id -cne $State.Fence.RuntimeId) { throw 'maintenance_foreign_runtime_identity' }
        [IO.File]::Delete($DrainIdentityPath)
    }
    Assert-CdrMarkerOwner $StopPath $State.Operation
    if ([IO.File]::Exists($StopPath)) { [IO.File]::Delete($StopPath) }
    if (-not [IO.File]::Exists($DisablePath)) { throw 'maintenance_seal_missing' }
    Clear-DeadRuntimeArtifacts -PreserveRestartDrain
    $null = New-CdrLaunchJournal $journalPath ('maintenance:'+$State.Operation) $State.Fence $State.CandidateHash
    $script:CdrLaunchJournalPath = $journalPath
    try {
        Assert-CdrMaintenanceArtifacts $State
        Assert-CdrMaintenanceDeadline $State
        Start-RustRuntime -DeadlineUtc ([DateTimeOffset]::Parse($State.Deadline))
    }
    finally { $script:CdrLaunchJournalPath = $null }
    $journal = Read-CdrLaunchJournal $journalPath
    if ($journal.Phase -ne 'child') { throw 'maintenance_launch_outcome_unknown' }
    if (-not (Get-CdrRecordedChild $journal)) { throw 'maintenance_recorded_child_dead' }
    # Included in the existing phase save/receipt; no UI I/O or extra state write.
    # A reentry does not invent this intent for an old or uncertain launch.
    if ($script:CdrTrayStartedIdentity -and $script:CdrTrayStartedIdentity -ceq $journal.ChildIdentity) {
        $State | Add-Member NoteProperty TrayBootstrapIdentity $journal.ChildIdentity -Force
    }
}

function Get-CdrMaintenanceChild($State, [switch]$RequireFresh) {
    $journal = Read-CdrLaunchJournal (Join-Path $State.Bundle 'launch.json')
    if (-not $journal -or $journal.Operation -cne ('maintenance:'+$State.Operation) -or
        $journal.ArtifactHash -cne $State.CandidateHash -or $journal.Phase -ne 'child') {
        throw 'maintenance_launch_outcome_unknown'
    }
    $child = Get-CdrRecordedChild $journal
    if (-not $child) { throw 'maintenance_recorded_child_dead' }
    if ((Get-VerifiedRuntimeIdentity) -cne $journal.ChildIdentity) { throw 'maintenance_child_lock_mismatch' }
    $instances = @(Get-Process -Name cdr-runtime -ErrorAction SilentlyContinue)
    if ($instances.Count -ne 1 -or $instances[0].Id -ne $child.Id) { throw 'maintenance_not_one_runtime' }
    if ($RequireFresh) {
        $health = Get-HeartbeatHealth $child
        if (-not $health.Healthy -or $health.Bootstrap) { throw 'maintenance_child_heartbeat_not_fresh' }
    }
    return $child
}

function Wait-CdrMaintenanceHeartbeats($State, [string]$StatePath) {
    $last = $null; $deadline = [DateTimeOffset]::UtcNow.AddSeconds((Get-CdrMaintenanceRemainingSeconds $State 45))
    do {
        $child = Get-CdrMaintenanceChild $State
        $health = Get-HeartbeatHealth $child
        if ($health.Healthy -and -not $health.Bootstrap) {
            $raw = [IO.File]::ReadAllText($HeartbeatPath)
            if ($raw -match '(?m)^updated_at=(\d+)\r?$') {
                $stamp = [long]$Matches[1]
                if ($null -ne $last -and $stamp -gt $last) {
                    $State.Heartbeats = @($last,$stamp)
                    Save-CdrMaintenanceState $State $StatePath
                    return
                }
                $last = $stamp
            }
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw 'maintenance_two_distinct_heartbeats_missing'
}

function Complete-CdrMaintenance($State, [string]$StatePath) {
    if($State.CompletionPolicy -ceq 'runtime-proof-v1'){Complete-CdrRuntimeMaintenance $State $StatePath;return}
    $null = Get-CdrMaintenanceChild $State -RequireFresh
    Assert-CdrMaintenanceArtifacts $State
    Assert-CdrMaintenanceMarkers $State
    if ($State.DiscordReceipt -notmatch '^\d+$' -or $State.Heartbeats.Count -ne 2 -or
        $State.Heartbeats[1] -le $State.Heartbeats[0]) { throw 'maintenance_completion_evidence_missing' }
    if ((Read-CdrMaintenanceState $StatePath).Operation -cne $State.Operation) {
        throw 'maintenance_owner_changed; completion refused'
    }
    Write-AtomicRestartMarker ($StatePath+'.completed') ($State | ConvertTo-Json -Depth 10 -Compress)
    Assert-CdrMarkerOwner $DisablePath $State.Operation
    if ([IO.File]::Exists($DisablePath)) { [IO.File]::Delete($DisablePath) }
    # State last: a crash before this delete leaves a repeatable verified completion.
    [IO.File]::Delete($StatePath)
    Write-Output ('maintenance_verified discord_receipt='+$State.DiscordReceipt)
}
