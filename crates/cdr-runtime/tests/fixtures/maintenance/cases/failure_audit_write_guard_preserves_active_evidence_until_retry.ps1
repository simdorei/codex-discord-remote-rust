param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'), [Text.Encoding]::UTF8)))
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture uncertainty')}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}finally{$guard.Dispose()}
$failed=Read-CdrMaintenanceState $StatePath
if($failed.FailureNoticePhase -cne 'unknown'){throw 'failure adapter was not exercised'}
$originalError=$failed.LastError
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$guard=[IO.File]::Open($auditPath,'Open','Read','None')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'AUDIT_LOCK_IGNORED'}catch{if($_.Exception.Message -eq 'AUDIT_LOCK_IGNORED'){throw}}}
finally{$guard.Dispose()}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'audit failure erased active evidence'}
if((Read-CdrMaintenanceState $StatePath).FirstCompletionFailure.LastError -cne $originalError){throw 'audit reentry overwrote first error'}
Invoke-CdrMaintenanceEngine $StatePath $op
$audit=Get-Content $auditPath -Raw|ConvertFrom-Json
if((Test-Path $StatePath) -or $audit.First.LastError -cne $originalError -or $audit.Latest.FailureNoticePhase -cne 'unknown'){throw 'audit retry lost original/unknown evidence'}
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'audit retry repeated external work'}
