param([string]$Variant)
switch ($Variant) {
'0' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceNotification.ps1')
$s=Read-CdrMaintenanceState $StatePath
$s|Add-Member FailureNoticePhase 'none';$s|Add-Member FailureReceipt '';$s|Add-Member FailureNoticeError ''
$script:posts=0
function Send-CdrMaintenanceMessage {$script:posts++;throw 'fixture transport uncertainty'}
Publish-CdrMaintenanceFailure $s $StatePath
$s=Read-CdrMaintenanceState $StatePath
Publish-CdrMaintenanceFailure $s $StatePath
if($script:posts -ne 1 -or $s.FailureNoticePhase -ne 'unknown'){throw 'failure notice replayed or lost'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
