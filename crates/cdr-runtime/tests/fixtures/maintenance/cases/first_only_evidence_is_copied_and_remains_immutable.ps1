param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.LastError='first error'
$s|Add-Member FirstCompletionFailure (Get-CdrCompletionFailureSnapshot $s)
$s.LastError=''
Save-CdrCompletionFailureAudit $s -FromActiveState
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$copy=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
if($copy.First.LastError -cne 'first error'){throw 'first-only evidence not copied'}
$s.LastError='current error'
$s.FirstCompletionFailure=$null
Save-CdrCompletionFailureAudit $s -FromActiveState
$copy=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
if($copy.First.LastError -cne 'first error' -or $copy.Snapshot.LastError -cne 'current error'){throw 'first overwritten by empty state'}
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
$s.FirstCompletionFailure=Get-CdrCompletionFailureSnapshot $s
try{Save-CdrCompletionFailureAudit $s -FromActiveState;throw 'first conflict accepted'}
catch{if($_.Exception.Message -notmatch 'completion_failure_first_conflict'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne $before){throw 'first conflict overwrote evidence'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
