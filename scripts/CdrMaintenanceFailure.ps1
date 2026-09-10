# Only public-safe codes and tri-state observations leave this machine.
function Get-CdrMaintenanceFailureObservation($State) {
    $original='unknown';$ack='unknown';$stop='unknown'
    try {
        $id=[int]$State.Fence.ProcessIdentity.Split('|')[0]
        $p=Get-Process -Id $id -ErrorAction Stop
        if (-not $p) { $original='exited' }
        elseif ((Get-RustProcessIdentity $p) -ceq $State.Fence.ProcessIdentity) { $original='alive' }
    } catch {
        if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) { $original='exited' }
        else { $original='unknown' }
    }
    try {
        $ack='missing'
        if (Get-Item -LiteralPath $DrainAckPath -ErrorAction Stop) {
            $ack='foreign'
            if (Test-RestartDrainFenceMatch (Get-RestartDrainFence $DrainAckPath -RequireSealed) $State.Fence) { $ack='matching' }
        }
    } catch {
        if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) { $ack='missing' }
        else { $ack='unknown' }
    }
    try {
        $stop='missing'
        if (Get-Item -LiteralPath $StopPath -ErrorAction Stop) {
            $stop='foreign'
            if ([IO.File]::ReadAllText($StopPath) -ceq $State.Operation) { $stop='owned' }
        }
    } catch {
        if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) { $stop='missing' }
        else { $stop='unknown' }
    }
    $code='maintenance_step_failed'
    foreach ($known in @('restart_drain_ack_timeout','graceful_exit_timeout','maintenance_backup_receipt_not_bound',
        'maintenance_backup_file_changed_or_wrong_path','maintenance_command_deadline_outcome_unknown',
        'maintenance_original_PID_still_present','maintenance_native_command_failed')) {
        if ($State.LastError.Contains($known)) { $code=$known;break }
    }
    return [pscustomobject]@{Code=$code;Original=$original;Ack=$ack;Stop=$stop;ObservedAt=[DateTimeOffset]::UtcNow.ToString('o')}
}
