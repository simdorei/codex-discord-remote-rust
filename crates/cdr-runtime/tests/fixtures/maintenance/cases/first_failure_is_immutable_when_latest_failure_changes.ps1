param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'), [Text.Encoding]::UTF8)))
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture uncertainty')}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}finally{$guard.Dispose()}
$failed=Read-CdrMaintenanceState $StatePath
$originalError=$failed.LastError
$failed.LastError='second cleanup failure';Save-CdrMaintenanceState $failed $StatePath
Invoke-CdrMaintenanceEngine $StatePath $op
$audit=Get-Content (Join-Path $s.Bundle 'completion-failure.json') -Raw|ConvertFrom-Json
$activeCopy=Get-Content (Join-Path $s.Bundle 'completion-active-failure.json') -Raw -Encoding UTF8|ConvertFrom-Json
if($audit.First.LastError -cne $originalError -or $activeCopy.Snapshot.LastError -cne 'second cleanup failure' -or
   $audit.Latest.FailureNoticePhase -cne 'unknown'){throw 'first/latest evidence contract lost'}
if($script:posts -ne 1){throw 'failure POST replayed'}
