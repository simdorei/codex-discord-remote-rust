param([string]$Variant)
switch ($Variant) {
'0' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceNotification.ps1')
function Get-CdrMaintenanceChild {[pscustomobject]@{Id=777}}
$script:notice=''
function Send-CdrMaintenanceMessage($s,$content,$nonce,$timeout) {$script:notice=$content;return '12345'}
$s=Read-CdrMaintenanceState $StatePath
$receipt=Send-CdrMaintenanceResult $s $StatePath
if($receipt -cne '12345'){throw 'delivery receipt changed'}
if($script:notice.Contains('cleanup')){throw 'update-only ticket falsely claims mirror cleanup'}
if(-not $script:notice.Contains('PID=777') -or -not $script:notice.Contains('two fresh heartbeats') -or
   -not $script:notice.Contains($s.CandidateHash)){throw 'binary proof absent from update notice'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
