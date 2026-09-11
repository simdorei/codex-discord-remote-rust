param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'), [Text.Encoding]::UTF8)))
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture transport uncertainty')}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'FAULT_NOT_INJECTED'}catch{if($_.Exception.Message -eq 'FAULT_NOT_INJECTED'){throw}}}
finally{$guard.Dispose()}
$failed=Read-CdrMaintenanceState $StatePath
if($failed.Phase -cne 'verified' -or -not $failed.LastError -or $failed.FailureNoticePhase -cne 'unknown' -or $script:posts -ne 1){throw 'actual engine/failure POST path not exercised'}
$originalError=$failed.LastError
Invoke-CdrMaintenanceEngine $StatePath $op
if((Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'owned cleanup did not finish'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
if(-not (Test-Path $auditPath)){throw 'R2: completion cleanup erased its failure evidence'}
$audit=Get-Content $auditPath -Raw|ConvertFrom-Json
if($audit.Operation -cne $op -or $audit.First.LastError -cne $originalError -or
   $audit.Latest.FailureNoticePhase -cne 'unknown' -or -not $audit.Latest.FailureNoticeError){throw 'first failure/unknown POST evidence lost'}
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'cleanup reentry replayed POST or launch'}
