param([string]$Variant)
$script:fail=$Variant
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'fault not injected'}catch{if($_.Exception.Message -notmatch '^injected_'){throw}}
if(@($script:calls|Where-Object{$_ -like '*:launch'}).Count){throw 'launch before failed precondition'}
Invoke-CdrMaintenanceEngine $StatePath $op
if((Read-CdrMaintenanceState $StatePath).Phase -ne 'verified'){throw 'did not resume'}
if(@($script:calls|Where-Object{$_ -like '*:launch'}).Count -ne 1){throw 'wrong launch count'}
