param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BACKUP.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
Invoke-CdrMaintenancePreStopBackup $s $StatePath
$saved=Read-CdrMaintenanceState $StatePath
Assert-CdrMaintenancePreStopBackup $saved
if($saved.PreStopBackup.Operation -cne $op -or -not $saved.PreStopBackup.PackageVerified){throw 'receipt not committed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
