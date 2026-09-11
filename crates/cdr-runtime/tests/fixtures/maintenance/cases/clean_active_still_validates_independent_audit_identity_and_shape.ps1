param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.LastError='independently captured error'
Save-CdrCompletionFailureAudit $s -ActiveStateUnpersisted
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$broken=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
$broken.Operation='f'*32
Write-AtomicRestartMarker $auditPath ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s;throw 'clean active bypassed audit validation'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath)) -cne $before){throw 'invalid audit was overwritten'}
}
'1' {
$s.LastError='independently captured error'
Save-CdrCompletionFailureAudit $s -ActiveStateUnpersisted
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$broken=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
$broken.Latest=$null
Write-AtomicRestartMarker $auditPath ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s;throw 'clean active bypassed audit validation'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath)) -cne $before){throw 'invalid audit was overwritten'}
}
'2' {
$s.LastError='independently captured error'
Save-CdrCompletionFailureAudit $s -ActiveStateUnpersisted
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$broken=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
$broken.ActiveStateUnpersisted='false'
Write-AtomicRestartMarker $auditPath ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s;throw 'clean active bypassed audit validation'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath)) -cne $before){throw 'invalid audit was overwritten'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
