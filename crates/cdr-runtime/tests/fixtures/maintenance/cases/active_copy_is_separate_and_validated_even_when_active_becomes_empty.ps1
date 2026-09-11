param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.LastError='persisted only in active state'
Save-CdrCompletionFailureAudit $s -FromActiveState
if(Test-Path (Join-Path $s.Bundle 'completion-failure.json')){throw 'disk snapshot was promoted to a new failure event'}
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$broken=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
$broken.Operation='f'*32
Write-AtomicRestartMarker $path ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s -FromActiveState;throw 'active copy validation bypassed'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne $before){throw 'invalid active copy overwritten'}
}
'1' {
$s.LastError='persisted only in active state'
Save-CdrCompletionFailureAudit $s -FromActiveState
if(Test-Path (Join-Path $s.Bundle 'completion-failure.json')){throw 'disk snapshot was promoted to a new failure event'}
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$broken=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
$broken.Snapshot=$null
Write-AtomicRestartMarker $path ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s -FromActiveState;throw 'active copy validation bypassed'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne $before){throw 'invalid active copy overwritten'}
}
'2' {
$s.LastError='persisted only in active state'
Save-CdrCompletionFailureAudit $s -FromActiveState
if(Test-Path (Join-Path $s.Bundle 'completion-failure.json')){throw 'disk snapshot was promoted to a new failure event'}
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$broken=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
$broken.First='invalid'
Write-AtomicRestartMarker $path ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s -FromActiveState;throw 'active copy validation bypassed'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne $before){throw 'invalid active copy overwritten'}
}
'3' {
$s.LastError='persisted only in active state'
Save-CdrCompletionFailureAudit $s -FromActiveState
if(Test-Path (Join-Path $s.Bundle 'completion-failure.json')){throw 'disk snapshot was promoted to a new failure event'}
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$broken=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
$broken.PSObject.Properties.Remove('First')
Write-AtomicRestartMarker $path ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s -FromActiveState;throw 'active copy validation bypassed'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne $before){throw 'invalid active copy overwritten'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
