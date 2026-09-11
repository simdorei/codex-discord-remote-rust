param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$script:activeGuard=$null
function Invoke-RestMethod {
 $script:posts++
 $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 throw [TimeoutException]::new('fixture uncertainty after request')
}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}
finally{$guard.Dispose();if($script:activeGuard){$script:activeGuard.Dispose()}}
if((Read-CdrMaintenanceState $StatePath).FailureNoticePhase -cne 'sending'){throw 'unknown result state-write failure not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if($before.Latest.FailureNoticePhase -cne 'unknown' -or -not $before.Latest.FailureNoticeError){throw 'unknown result not archived'}
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if($after.Latest.FailureNoticePhase -cne 'unknown' -or $after.Latest.FailureNoticeError -cne $before.Latest.FailureNoticeError){throw 'unknown failure result downgraded on cleanup'}
if((Test-Path $StatePath) -or $script:posts -ne 1){throw 'unknown audit cleanup replayed or stayed blocked'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
