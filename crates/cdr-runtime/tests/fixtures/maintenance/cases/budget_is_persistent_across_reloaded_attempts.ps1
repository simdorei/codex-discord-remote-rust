param([string]$Variant)
function Invoke-CdrMaintenancePreflight {Effect 'preflight';throw 'retryable_preflight_failure'}
1..3|ForEach-Object{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}
$before=$script:calls.Count
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'budget reset'}catch{if($_.Exception.Message -notmatch 'budget_exhausted'){throw}}
$s=Read-CdrMaintenanceState $StatePath
if($s.Attempts -ne 3 -or -not $s.Halted -or $script:calls.Count -ne $before){throw 'budget not durable'}
if(@($script:calls|Where-Object{$_ -like '*:stop'}).Count){throw 'stop before ACK'}
