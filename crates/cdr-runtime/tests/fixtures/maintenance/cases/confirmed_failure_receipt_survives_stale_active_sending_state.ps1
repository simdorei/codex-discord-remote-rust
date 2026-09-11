param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$script:activeGuard=$null
function Invoke-RestMethod {
 $script:posts++
 $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 [pscustomobject]@{id='12345';channel_id=$s.NotifyChannel}
}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}
finally{$guard.Dispose();if($script:activeGuard){$script:activeGuard.Dispose()}}
if((Read-CdrMaintenanceState $StatePath).FailureNoticePhase -cne 'sending'){throw 'post-result state-write failure not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if($before.Latest.FailureNoticePhase -cne 'sent' -or $before.Latest.FailureReceipt -cne '12345'){throw 'confirmed failure POST not archived'}
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if($after.Latest.FailureNoticePhase -cne 'sent' -or $after.Latest.FailureReceipt -cne '12345'){throw 'confirmed failure receipt downgraded on cleanup'}
if((Test-Path $StatePath) -or $script:posts -ne 1){throw 'audit completion replayed POST or stayed blocked'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
