param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
[void][IO.Directory]::CreateDirectory($auditPath)
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}finally{$guard.Dispose()}
$first=(Read-CdrMaintenanceState $StatePath).FirstCompletionFailure.LastError
if(-not $first -or [IO.File]::Exists($auditPath)){throw 'first active-only failure not exercised'}
[IO.File]::WriteAllText($StopPath,$op)
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'stop intent ignored'}
catch{if($_.Exception.Message -notmatch 'post_launch_intent'){throw}}
$later=(Read-CdrMaintenanceState $StatePath).LastError
if($first -ceq $later){throw 'distinct second failure not exercised'}
[IO.File]::Delete($StopPath)
Invoke-CdrMaintenanceEngine $StatePath $op
$copy=Get-Content (Join-Path $s.Bundle 'completion-active-failure.json') -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $copy.Snapshot.LastError -cne $later -or $copy.First.LastError -cne $first){throw 'active-only first failure lost during cleanup'}
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'active-only recovery replayed external work'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
