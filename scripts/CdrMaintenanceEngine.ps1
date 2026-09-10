# Completion failures share one recorder on initial execution and verified reentry.
function Invoke-CdrMaintenanceFailureHandling($state, [string]$StatePath, $primary, [switch]$CompletionReentry) {
    $state.LastError = $primary.Exception.Message
    # A recorded shutdown failure cannot be converted into late success on retry.
    if ($state.Phase -in @('prepared','drained','stop_requested')) { $state.Halted=$true }
    $state.FailureObservation=Get-CdrMaintenanceFailureObservation $state
    if($state.CompletionPolicy -ceq 'runtime-proof-v1' -and $state.Phase -ceq 'verified' -and
       -not $state.FirstCompletionFailure){
        $state|Add-Member FirstCompletionFailure (Get-CdrCompletionFailureSnapshot $state)
    }
    # Unknown irreversible outcomes and dead children are terminal, not retries.
    if ($state.LastError -match 'unconfirmed|outcome_unknown|recorded_child_dead|owner_changed|foreign|T1_' -or
        $state.Attempts -ge 3 -or [DateTimeOffset]::UtcNow -ge [DateTimeOffset]::Parse($state.Deadline)) {
        $state.Halted = $true
    }
    try { Save-CdrMaintenanceState $state $StatePath }
    catch {
        $preservationFailure=$_;$auditResult='not_attempted'
        if($state.CompletionPolicy -ceq 'runtime-proof-v1' -and $state.Phase -ceq 'verified'){
            try {Save-CdrCompletionFailureAudit $state -ActiveStateUnpersisted;$auditResult='saved'}
            catch {$auditResult='failed: '+$_.Exception.Message}
        }
        # Do not POST or continue cleanup after the active save failure. Record
        # the bound operation's in-memory failure independently, then surface both.
        throw "Maintenance failed: $($primary.Exception.Message); state preservation failed: $($preservationFailure.Exception.Message); independent completion audit=$auditResult"
    }
    if(-not $CompletionReentry){
        try { Publish-CdrMaintenanceFailure $state $StatePath }
        catch { Write-Warning 'Maintenance failure notice pending locally; original error retained' }
    }
    if($state.CompletionPolicy -ceq 'runtime-proof-v1' -and $state.Phase -ceq 'verified'){
        try {Save-CdrCompletionFailureAudit $state}
        catch {Write-Warning 'Completion failure audit unavailable; active evidence retained; cleanup must verify the audit before deletion'}
    }
    throw $primary
}

# Real OS actions live in companion modules. No generic watchdog/recovery fallback.
function Invoke-CdrMaintenanceEngine([string]$StatePath, [string]$ExpectedOperation) {
    $state = Read-CdrMaintenanceState $StatePath
    if ($ExpectedOperation -notmatch '^[a-f0-9]{32}$' -or $state.Operation -cne $ExpectedOperation) {
        throw 'maintenance_expected_operation_mismatch; no state or process action'
    }
    if ($state.Phase -eq 'verified') {
        try {Complete-CdrMaintenance $state $StatePath}
        catch {
            if($state.CompletionPolicy -cne 'runtime-proof-v1'){throw}
            Invoke-CdrMaintenanceFailureHandling $state $StatePath $_ -CompletionReentry
        }
        return
    }
    try { Assert-CdrMaintenanceBudget $state }
    catch {
        if (-not $state.Halted) {
            $state.Halted=$true; $state.LastError=$_.Exception.Message
            $state.FailureObservation=Get-CdrMaintenanceFailureObservation $state
            Save-CdrMaintenanceState $state $StatePath
        }
        try { Publish-CdrMaintenanceFailure $state $StatePath }
        catch { Write-Warning 'Maintenance failure notice pending locally' }
        throw
    }
    $state.Attempts++
    Save-CdrMaintenanceState $state $StatePath
    try {
        Assert-CdrMaintenanceArtifacts $state
        Assert-CdrMaintenanceMarkers $state
        if ($null -ne $state.ActiveCommand) { throw 'maintenance_command_outcome_unknown; no helper replay' }
        while ($state.Phase -ne 'verified') {
            Assert-CdrMaintenanceArtifacts $state
            Assert-CdrMaintenanceMarkers $state
            if ([DateTimeOffset]::UtcNow -ge [DateTimeOffset]::Parse($state.Deadline)) {
                throw 'maintenance_deadline_exceeded'
            }
            switch ($state.Phase) {
                'planned' {
                    Assert-CdrMaintenanceShutdownPolicy $state.ShutdownPolicy
                    Assert-CdrMaintenanceRecoveryArmed $state
                    Invoke-CdrMaintenancePreflight $state
                    Invoke-CdrMaintenancePreStopBackup $state $StatePath
                    Invoke-CdrMaintenancePreflight $state
                    Set-CdrMaintenancePhase $state $StatePath 'prepared'
                }
                'prepared' {
                    Assert-CdrMaintenanceShutdownPolicy $state.ShutdownPolicy
                    Assert-CdrMaintenancePreStopBackup $state
                    Invoke-CdrMaintenancePreflight $state
                    Invoke-CdrMaintenanceDrain $state
                    Set-CdrMaintenancePhase $state $StatePath 'drained'
                }
                'drained' {
                    Assert-RestartDrainBound $state.Fence $state.Fence.ProcessIdentity
                    Invoke-CdrMaintenancePreflight $state
                    Assert-RestartDrainBound $state.Fence $state.Fence.ProcessIdentity
                    Set-CdrMaintenancePhase $state $StatePath 'stop_requested'
                }
                'stop_requested' {
                    Invoke-CdrMaintenanceStop $state
                    Set-CdrMaintenancePhase $state $StatePath 'stopped'
                }
                'stopped' {
                    Assert-CdrMaintenanceNoRuntime $state
                    Invoke-CdrMaintenancePackaging $state
                    Set-CdrMaintenancePhase $state $StatePath 'installing'
                }
                'installing' {
                    Install-CdrMaintenanceCandidate $state
                    Set-CdrMaintenancePhase $state $StatePath 'candidate_installed'
                }
                'candidate_installed' {
                    Assert-CdrMaintenanceNoRuntime $state
                    # Durable BEFORE the operator can initialize or mutate any DB.
                    Set-CdrMaintenancePhase $state $StatePath 'mutation_started'
                }
                'mutation_started' {
                    Assert-CdrMaintenanceNoRuntime $state
                    Invoke-CdrMaintenanceCleanup $state
                    Set-CdrMaintenancePhase $state $StatePath 'cleanup_confirmed'
                }
                'cleanup_confirmed' {
                    Assert-CdrMaintenanceNoRuntime $state
                    Invoke-CdrMaintenanceFullReadiness $state
                    Set-CdrMaintenancePhase $state $StatePath 'readiness_verified'
                }
                'readiness_verified' {
                    Assert-CdrMaintenanceNoRuntime $state
                    Invoke-CdrMaintenanceFullReadiness $state
                    # This durable phase authorizes resumable removal of ONLY our old markers.
                    Set-CdrMaintenancePhase $state $StatePath 'launch_ready'
                }
                'launch_ready' {
                    Invoke-CdrMaintenanceLaunch $state $StatePath
                    Set-CdrMaintenancePhase $state $StatePath 'launched'
                }
                'launched' {
                    Wait-CdrMaintenanceHeartbeats $state $StatePath
                    Set-CdrMaintenancePhase $state $StatePath 'healthy'
                }
                'healthy' {
                    Wait-CdrMaintenanceHeartbeats $state $StatePath
                    if($state.CompletionPolicy -ceq 'runtime-proof-v1') {
                        $state|Add-Member RuntimeEvidence (Get-CdrRuntimeCompletionEvidence $state) -Force
                        $state|Add-Member InitialNotificationOutcome 'pending' -Force
                        Set-CdrMaintenancePhase $state $StatePath 'verified'
                        break
                    }
                    Set-CdrMaintenancePhase $state $StatePath 'notifying'
                    # A failed/unknown POST is NOT permission to launch/delete/send again.
                    $state.DiscordReceipt = Send-CdrMaintenanceResult $state $StatePath
                    Set-CdrMaintenancePhase $state $StatePath 'verified'
                }
                'notifying' { throw 'maintenance_notification_outcome_unknown; inspect Discord, no automatic resend' }
                default { throw 'maintenance_phase_invalid' }
            }
        }
        Complete-CdrMaintenance $state $StatePath
    } catch { Invoke-CdrMaintenanceFailureHandling $state $StatePath $_ }

    # Notification failures cannot reenter the deployment failure/rollback path.
    if($state.CompletionPolicy -ceq 'runtime-proof-v1') {
        try { Send-CdrCompletedNotice $state }
        catch { Write-CdrCompletedNoticeFailure $state $_ }
    }
}
