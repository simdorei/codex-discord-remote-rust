param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$copyPath=Join-Path $s.Bundle 'completion-active-failure.json'
[void][IO.Directory]::CreateDirectory($auditPath)
[void][IO.Directory]::CreateDirectory($copyPath)
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture unknown result')}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}finally{$guard.Dispose()}
$first=(Read-CdrMaintenanceState $StatePath).FirstCompletionFailure
if(-not $first.LastError -or -not $first.FailureObservation){throw 'first failure not observed'}
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'copy creation failure ignored'}catch{}
$active=Read-CdrMaintenanceState $StatePath
$second=$active.LastError
if($first.LastError -ceq $second -or $active.FailureNoticePhase -cne 'unknown'){throw 'distinct second failure or unknown not exercised'}
# Remove only these two empty fixture directories, not runtime/user files.
[IO.Directory]::Delete($auditPath)
[IO.Directory]::Delete($copyPath)
Invoke-CdrMaintenanceEngine $StatePath $op
$copy=Get-Content $copyPath -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $copy.Snapshot.LastError -cne $second -or
   ($copy.First|ConvertTo-Json -Depth 10 -Compress) -cne ($first|ConvertTo-Json -Depth 10 -Compress) -or
   $copy.Snapshot.FailureNoticePhase -cne 'unknown' -or
   $copy.Snapshot.FailureNoticeError -cne $active.FailureNoticeError){throw 'active-only first/current/unknown evidence lost'}
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'recovery replayed external work'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
