$s=Read-CdrMaintenanceState $StatePath
$s|Add-Member ShutdownPolicy 'live-handshake-v1' -Force
Save-CdrMaintenanceState $s $StatePath
function Invoke-CdrMaintenancePreStopBackup {Effect 'backup'}
function Assert-CdrMaintenancePreStopBackup {Effect 'backup_bound'}
