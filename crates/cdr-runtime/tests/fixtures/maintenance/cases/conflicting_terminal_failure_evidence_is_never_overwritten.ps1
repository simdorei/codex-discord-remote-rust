param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.LastError='fixture completion failure';$s.FailureNoticePhase='sent';$s.FailureReceipt='12345'
Save-CdrCompletionFailureAudit $s
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath))
$s.FailureNoticePhase='unknown';$s.FailureReceipt=''
try{Save-CdrCompletionFailureAudit $s;throw 'conflicting terminal evidence accepted'}
catch{if($_.Exception.Message -notmatch 'failure_notice_conflict'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath)) -cne $before){throw 'conflicting terminal evidence overwritten'}
}
'1' {
$s.LastError='fixture completion failure';$s.FailureNoticePhase='sent';$s.FailureReceipt='12345'
Save-CdrCompletionFailureAudit $s
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath))
$s.FailureNoticePhase='sent';$s.FailureReceipt='67890'
try{Save-CdrCompletionFailureAudit $s;throw 'conflicting terminal evidence accepted'}
catch{if($_.Exception.Message -notmatch 'failure_notice_conflict'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath)) -cne $before){throw 'conflicting terminal evidence overwritten'}
}
'2' {
$s.LastError='fixture completion failure';$s.FailureNoticePhase='unknown';$s.FailureReceipt=''
Save-CdrCompletionFailureAudit $s
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath))
$s.FailureNoticePhase='sent';$s.FailureReceipt='12345'
try{Save-CdrCompletionFailureAudit $s;throw 'conflicting terminal evidence accepted'}
catch{if($_.Exception.Message -notmatch 'failure_notice_conflict'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath)) -cne $before){throw 'conflicting terminal evidence overwritten'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
