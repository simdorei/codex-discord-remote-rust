param([string]$Variant)
$script:fail='notify'
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'unknown accepted'}catch{if($_.Exception.Message -notmatch 'notification_outcome_unknown'){throw}}
foreach($action in @('launch','cleanup','notify')){
 if(@($script:calls|Where-Object{$_ -like "*:$action"}).Count -ne 1){throw "replayed $action"}
}
if(-not (Read-CdrMaintenanceState $StatePath).Halted){throw 'unknown not halted'}
