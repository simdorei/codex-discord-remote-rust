param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BACKUP.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
Invoke-CdrMaintenancePreStopBackup $s $StatePath
try{New-CdrMaintenanceSnapshotReceipt $s $StatePath 'PostStopBackup';throw 'stale snapshot accepted'}
catch{if($_.Exception.Message -ne 'maintenance_post_stop_snapshot_not_new'){throw}}
if($null -ne (Read-CdrMaintenanceState $StatePath).PostStopBackup){throw 'stale receipt committed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
