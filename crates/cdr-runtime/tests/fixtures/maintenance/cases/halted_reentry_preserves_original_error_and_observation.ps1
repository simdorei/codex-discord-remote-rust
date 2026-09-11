param([string]$Variant)
switch ($Variant) {
'0' {
$script:fail='ACK'
$s=Read-CdrMaintenanceState $StatePath
$s|Add-Member FailureNoticePhase 'none';Save-CdrMaintenanceState $s $StatePath
$script:posts=0;$script:original='alive'
function Get-CdrMaintenanceFailureObservation {[pscustomobject]@{Original=$script:original;Code='original_error'}}
function Publish-CdrMaintenanceFailure {param($s,$p)
 if($s.FailureNoticePhase -ne 'none'){return}
 $script:posts++;$s.FailureNoticePhase='unknown';Save-CdrMaintenanceState $s $p
}
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}
$first=Read-CdrMaintenanceState $StatePath
if(-not $first.Halted){throw 'fixture did not halt'}
$errorText=$first.LastError;$observation=$first.FailureObservation|ConvertTo-Json -Compress
$script:original='exited'
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}
$second=Read-CdrMaintenanceState $StatePath
if($second.LastError -cne $errorText -or ($second.FailureObservation|ConvertTo-Json -Compress) -cne $observation){throw 'terminal evidence overwritten'}
if($second.FailureNoticePhase -cne 'unknown' -or $script:posts -ne 1){throw 'unknown notice replayed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
