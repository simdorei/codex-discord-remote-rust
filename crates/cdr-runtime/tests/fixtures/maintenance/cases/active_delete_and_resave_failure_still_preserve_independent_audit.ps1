param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.LastError='';Save-CdrMaintenanceState $s $StatePath
if('healthy' -eq 'verified'){
 Wait-CdrMaintenanceHeartbeats $s $StatePath
 $s|Add-Member RuntimeEvidence (Get-CdrRuntimeCompletionEvidence $s)
 $s|Add-Member InitialNotificationOutcome 'pending'
 $s.Phase='verified';Save-CdrMaintenanceState $s $StatePath
}
$script:activeGuard=$null
function Get-HeartbeatHealth {
 if((Test-Path ($StatePath+'.completed')) -and -not $script:activeGuard){
  $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 }
 [pscustomobject]@{Healthy=$true;Bootstrap=$false}
}
$failure=''
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{$failure=$_.Exception.Message}
finally{if($script:activeGuard){$script:activeGuard.Dispose()}}
if($failure -notmatch 'state preservation failed' -or (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'R2-B real engine boundary not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
if(-not (Test-Path $auditPath)){throw 'R2-B independent audit skipped after active save error'}
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if(-not $before.Latest.LastError -or -not $failure.Contains($before.Latest.LastError) -or
   $before.Latest.LastError -ceq ''){throw 'R2-B new cleanup error not archived'}
$errorText=$before.Latest.LastError
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $after.Latest.LastError -cne $errorText){throw 'R2-B stale active erased independently preserved error'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'R2-B recovery replayed external work'}
}
'1' {
$s.LastError='previous preflight warning';Save-CdrMaintenanceState $s $StatePath
if('healthy' -eq 'verified'){
 Wait-CdrMaintenanceHeartbeats $s $StatePath
 $s|Add-Member RuntimeEvidence (Get-CdrRuntimeCompletionEvidence $s)
 $s|Add-Member InitialNotificationOutcome 'pending'
 $s.Phase='verified';Save-CdrMaintenanceState $s $StatePath
}
$script:activeGuard=$null
function Get-HeartbeatHealth {
 if((Test-Path ($StatePath+'.completed')) -and -not $script:activeGuard){
  $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 }
 [pscustomobject]@{Healthy=$true;Bootstrap=$false}
}
$failure=''
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{$failure=$_.Exception.Message}
finally{if($script:activeGuard){$script:activeGuard.Dispose()}}
if($failure -notmatch 'state preservation failed' -or (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'R2-B real engine boundary not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
if(-not (Test-Path $auditPath)){throw 'R2-B independent audit skipped after active save error'}
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if(-not $before.Latest.LastError -or -not $failure.Contains($before.Latest.LastError) -or
   $before.Latest.LastError -ceq 'previous preflight warning'){throw 'R2-B new cleanup error not archived'}
$errorText=$before.Latest.LastError
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $after.Latest.LastError -cne $errorText){throw 'R2-B stale active erased independently preserved error'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'R2-B recovery replayed external work'}
}
'2' {
$s.LastError='';Save-CdrMaintenanceState $s $StatePath
if('verified' -eq 'verified'){
 Wait-CdrMaintenanceHeartbeats $s $StatePath
 $s|Add-Member RuntimeEvidence (Get-CdrRuntimeCompletionEvidence $s)
 $s|Add-Member InitialNotificationOutcome 'pending'
 $s.Phase='verified';Save-CdrMaintenanceState $s $StatePath
}
$script:activeGuard=$null
function Get-HeartbeatHealth {
 if((Test-Path ($StatePath+'.completed')) -and -not $script:activeGuard){
  $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 }
 [pscustomobject]@{Healthy=$true;Bootstrap=$false}
}
$failure=''
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{$failure=$_.Exception.Message}
finally{if($script:activeGuard){$script:activeGuard.Dispose()}}
if($failure -notmatch 'state preservation failed' -or (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'R2-B real engine boundary not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
if(-not (Test-Path $auditPath)){throw 'R2-B independent audit skipped after active save error'}
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if(-not $before.Latest.LastError -or -not $failure.Contains($before.Latest.LastError) -or
   $before.Latest.LastError -ceq ''){throw 'R2-B new cleanup error not archived'}
$errorText=$before.Latest.LastError
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $after.Latest.LastError -cne $errorText){throw 'R2-B stale active erased independently preserved error'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'R2-B recovery replayed external work'}
}
'3' {
$s.LastError='previous preflight warning';Save-CdrMaintenanceState $s $StatePath
if('verified' -eq 'verified'){
 Wait-CdrMaintenanceHeartbeats $s $StatePath
 $s|Add-Member RuntimeEvidence (Get-CdrRuntimeCompletionEvidence $s)
 $s|Add-Member InitialNotificationOutcome 'pending'
 $s.Phase='verified';Save-CdrMaintenanceState $s $StatePath
}
$script:activeGuard=$null
function Get-HeartbeatHealth {
 if((Test-Path ($StatePath+'.completed')) -and -not $script:activeGuard){
  $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 }
 [pscustomobject]@{Healthy=$true;Bootstrap=$false}
}
$failure=''
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{$failure=$_.Exception.Message}
finally{if($script:activeGuard){$script:activeGuard.Dispose()}}
if($failure -notmatch 'state preservation failed' -or (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'R2-B real engine boundary not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
if(-not (Test-Path $auditPath)){throw 'R2-B independent audit skipped after active save error'}
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if(-not $before.Latest.LastError -or -not $failure.Contains($before.Latest.LastError) -or
   $before.Latest.LastError -ceq 'previous preflight warning'){throw 'R2-B new cleanup error not archived'}
$errorText=$before.Latest.LastError
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $after.Latest.LastError -cne $errorText){throw 'R2-B stale active erased independently preserved error'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'R2-B recovery replayed external work'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
