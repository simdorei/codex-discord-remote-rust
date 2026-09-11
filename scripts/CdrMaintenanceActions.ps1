function Assert-CdrMaintenanceArtifacts($State) {
    Assert-CdrMaintenanceProgramPins $State
    if ((Get-CdrArtifactHash $EnvPath) -cne $State.EnvHash) { throw 'maintenance_environment_changed' }
    foreach ($pair in @(@('CandidatePath','CandidateHash'),@('OperatorPath','OperatorHash'))) {
        if ((Get-CdrArtifactHash $State.($pair[0])) -cne $State.($pair[1])) {
            throw 'maintenance_artifact_changed'
        }
    }
    $hash = Get-CdrArtifactHash $BinaryPath
    $allowed = switch ($State.Phase) {
        {$_ -in @('planned','prepared','drained','stop_requested','stopped')} { @($State.BaselineHash) }
        'installing' { @($State.BaselineHash,$State.CandidateHash) }
        default { @($State.CandidateHash) }
    }
    if ($hash -cnotin $allowed) { throw 'maintenance_installed_hash_wrong_for_phase' }
    if ($State.Phase -ne 'planned') { Assert-CdrMaintenancePreStopBackup $State }
    if ($State.Phase -notin @('planned','prepared','drained','stop_requested','stopped')) {
        Assert-CdrMaintenanceBackupReceipt $State 'PostStopBackup'
    }
}

function Assert-CdrMaintenanceMarkers($State) {
    foreach ($path in @($StopPath,$DisablePath)) { Assert-CdrMarkerOwner $path $State.Operation }
    if ($State.Phase -notin @('planned','prepared','verified') -and -not [IO.File]::Exists($DisablePath)) {
        throw 'maintenance_seal_missing; no action'
    }
    if ($State.Phase -in @('planned','prepared','drained') -and [IO.File]::Exists($StopPath)) {
        throw 'maintenance_stop_before_authorized_phase; preserved'
    }
    if ($State.Phase -in @('launched','healthy','notifying','verified')) {
        foreach ($path in @($StopPath,$DrainPreparePath,$DrainAckPath)) {
            if ([IO.File]::Exists($path)) { throw 'maintenance_post_launch_intent_present; preserved' }
        }
    }
    foreach ($path in @($DrainPreparePath,$DrainAckPath)) {
        if ([IO.File]::Exists($path) -and
            -not (Test-RestartDrainFenceMatch (Get-RestartDrainFence $path) $State.Fence)) {
            throw 'maintenance_foreign_drain_marker; preserved'
        }
    }
    foreach ($name in @('.codex_discord_rust.restart','.codex_discord_rust.restart.launch',
        '.codex_discord_runtime.cutover')) {
        if ([IO.File]::Exists((Join-Path $RepoRoot $name))) { throw 'maintenance_foreign_operation; preserved' }
    }
    Assert-CdrNoOrphanRestartClaim $RepoRoot
}

function Invoke-CdrMaintenanceProbe($State, [string]$Mode) {
    if ((Get-CdrArtifactHash $State.OperatorPath) -cne $State.OperatorHash) { throw 'operator_hash_changed' }
    Invoke-CdrMaintenanceCommand $State $State.OperatorPath @($Mode,'--env',$EnvPath) 120
}
function Invoke-CdrMaintenancePreflight($State) { Invoke-CdrMaintenanceProbe $State 'preflight' }
function Invoke-CdrMaintenanceCleanup($State) { Invoke-CdrMaintenanceProbe $State 'cleanup' }

function Invoke-CdrMaintenanceDrain($State) {
    Assert-CdrMaintenancePreStopBackup $State
    if ((Get-VerifiedRuntimeIdentity) -cne $State.Fence.ProcessIdentity) { throw 'maintenance_original_changed' }
    $identity = Get-RuntimeDrainIdentity (Get-VerifiedRuntimeProcess) $State.Fence.ProcessIdentity
    if ($identity.RuntimeId -cne $State.Fence.RuntimeId) { throw 'maintenance_runtime_id_changed' }
    Assert-CdrMaintenanceMarkers $State
    Assert-CdrMaintenanceDeadline $State
    if (-not [IO.File]::Exists($DisablePath)) { Write-NewCdrMarker $DisablePath $State.Operation }
    if (-not [IO.File]::Exists($DrainPreparePath)) {
        $f = $State.Fence
        Assert-CdrMaintenanceDeadline $State
        Write-NewCdrMarker $DrainPreparePath (
            "version=1`nruntime_id=$($f.RuntimeId)`nprocess_identity=$($f.ProcessIdentity)`nnonce=$($f.Nonce)`n")
    }
    $fence = Enter-RestartDrain (Get-VerifiedRuntimeProcess) $State.Fence.ProcessIdentity (Get-CdrMaintenanceRemainingSeconds $State 120)
    if (-not (Test-RestartDrainFenceMatch $fence $State.Fence)) { throw 'maintenance_ACK_wrong_nonce' }
    Assert-RestartDrainBound $State.Fence $State.Fence.ProcessIdentity
    Assert-CdrMaintenanceDeadline $State
}

function Assert-CdrMaintenanceNoRuntime($State) {
    $oldPid = [int]($State.Fence.ProcessIdentity.Split('|')[0])
    if (Get-Process -Id $oldPid -ErrorAction SilentlyContinue) { throw 'maintenance_original_PID_still_present' }
    # Deliberately conservative across projects; never stop an unrelated runtime.
    if (Get-Process -Name cdr-runtime -ErrorAction SilentlyContinue) { throw 'maintenance_runtime_appeared' }
    $lockPid = Get-RuntimePid
    if ($lockPid -gt 0 -and (Get-Process -Id $lockPid -ErrorAction SilentlyContinue)) {
        throw 'maintenance_foreign_live_lock'
    }
}

function Invoke-CdrMaintenanceStop($State) {
    Assert-CdrMaintenanceMarkers $State
    $original = Get-Process -Id ([int]$State.Fence.ProcessIdentity.Split('|')[0]) -ErrorAction SilentlyContinue
    if ($original) {
        Assert-RestartDrainBound $State.Fence $State.Fence.ProcessIdentity
        Assert-CdrMaintenanceDeadline $State
        if (-not [IO.File]::Exists($StopPath)) { Write-NewCdrMarker $StopPath $State.Operation }
        Wait-RustRuntimeExit $original $State.Fence.ProcessIdentity 'maintenance_v2' (Get-CdrMaintenanceRemainingSeconds $State 45)
    }
    Assert-CdrMaintenanceNoRuntime $State
}

function Invoke-CdrMaintenancePackaging($State) {
    Assert-CdrMaintenanceNoRuntime $State
    Assert-CdrMaintenanceDeadline $State
    Assert-CdrMaintenancePreStopBackup $State
    New-CdrMaintenanceSnapshotReceipt $State (Get-CdrMaintenancePath $RepoRoot) 'PostStopBackup'
}

function Install-CdrMaintenanceCandidate($State) {
    Assert-CdrMaintenanceNoRuntime $State
    Assert-CdrMaintenanceArtifacts $State
    if ((Get-CdrArtifactHash $BinaryPath) -ceq $State.CandidateHash) { return }
    $next = $BinaryPath + '.maintenance.' + $State.Operation
    Assert-CdrMaintenanceDeadline $State
    if (-not [IO.File]::Exists($next)) { [IO.File]::Copy($State.CandidatePath,$next,$false) }
    if ((Get-CdrArtifactHash $next) -cne $State.CandidateHash) { throw 'maintenance_staging_hash_wrong' }
    Assert-CdrMaintenanceDeadline $State
    [IO.File]::Replace($next,$BinaryPath,[System.Management.Automation.Language.NullString]::Value)
    if ((Get-CdrArtifactHash $BinaryPath) -cne $State.CandidateHash) { throw 'maintenance_install_hash_wrong' }
}

function Invoke-CdrMaintenanceFullReadiness($State) {
    if ((Get-CdrArtifactHash $BinaryPath) -cne $State.CandidateHash) { throw 'maintenance_readiness_hash_wrong' }
    $seconds=Get-CdrMaintenanceRemainingSeconds $State 120
    # Match the runtime's 45s startup + 8s close and reserve 5s for scheduling.
    if ($seconds -le 58) { throw 'maintenance_readiness_budget_insufficient; no child started' }
    $waitSeconds=[math]::Min(60,$seconds-58)
    Invoke-CdrMaintenanceCommand $State $BinaryPath @('--restart-readiness','--restart-quiet-seconds','0',
        '--restart-wait-timeout-seconds',[string]$waitSeconds,'--env',$EnvPath) $seconds
}
