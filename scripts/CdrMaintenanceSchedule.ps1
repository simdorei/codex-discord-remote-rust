function Get-CdrMaintenanceTaskArguments([string]$StatePath, [string]$Operation) {
    if ($Operation -notmatch '^[a-f0-9]{32}$') { throw 'maintenance_expected_operation_invalid' }
    '-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "' +
        (Join-Path $RepoRoot 'scripts/Invoke-CdrMaintenance.ps1') + '" -StatePath "' + $StatePath + '" -ExpectedOperation ' + $Operation
}

function Get-CdrMaintenanceUserSid([string]$User) {
    if ([string]::IsNullOrWhiteSpace($User)) { throw 'maintenance_task_user_unresolved' }
    try {
        if ($User -match '^S-\d-') { return ([Security.Principal.SecurityIdentifier]::new($User)).Value }
        return ([Security.Principal.NTAccount]::new($User)).Translate([Security.Principal.SecurityIdentifier]).Value
    } catch { throw 'maintenance_task_user_unresolved' }
}

function Assert-CdrMaintenanceRecoveryArmed($State) {
    $task = Get-ScheduledTask -TaskName $State.TaskName -ErrorAction Stop
    $actions = @($task.Actions)
    if (-not $task.Settings.Enabled -or $actions.Count -ne 1 -or
        $actions[0].Execute -cne $State.PowerShellPath -or
        $actions[0].Arguments -cne (Get-CdrMaintenanceTaskArguments (Get-CdrMaintenancePath $RepoRoot) $State.Operation) -or
        $actions[0].WorkingDirectory -cne $RepoRoot -or
        (Get-CdrMaintenanceUserSid $task.Principal.UserId) -cne (Get-CdrMaintenanceUserSid $State.TaskUser)) {
        throw 'maintenance_independent_recovery_not_armed'
    }
    $repeating = @($task.Triggers | Where-Object {
        $_.Enabled -and $_.Repetition.Interval -eq 'PT1M' -and $_.Repetition.Duration -eq 'PT30M'
    })
    if ($repeating.Count -ne 1) { throw 'maintenance_recovery_trigger_invalid' }
}
