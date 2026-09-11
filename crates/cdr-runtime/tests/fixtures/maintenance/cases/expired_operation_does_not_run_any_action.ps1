param([string]$Variant)
$s=Read-CdrMaintenanceState $StatePath
$s.CreatedAt=$now.AddMinutes(-40).ToString('o');$s.Deadline=$now.AddMinutes(-10).ToString('o')
Save-CdrMaintenanceState $s $StatePath
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'expired accepted'}catch{if($_.Exception.Message -notmatch 'budget_exhausted'){throw}}
if($script:calls.Count){throw 'expired action'}
