param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BACKUP.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$script:nativeFailure='--backup-store'
try{Invoke-CdrMaintenancePreStopBackup $s $StatePath;throw 'fault not checked'}
catch{if($_.Exception.Message -ne 'injected_native_failure'){throw}}
if((Read-CdrMaintenanceState $StatePath).PreStopBackup.PackageVerified -eq $true){throw 'failed package certified'}
}
'1' {
$script:nativeFailure='--check-config'
try{Invoke-CdrMaintenancePreStopBackup $s $StatePath;throw 'fault not checked'}
catch{if($_.Exception.Message -ne 'injected_native_failure'){throw}}
if((Read-CdrMaintenanceState $StatePath).PreStopBackup.PackageVerified -eq $true){throw 'failed package certified'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
