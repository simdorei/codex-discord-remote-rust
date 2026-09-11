param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'LOCAL.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$script:fail='ACK'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'missing fault'}
catch{if($_.Exception.Message -notmatch '^injected_'){throw}}
if(-not (Read-CdrMaintenanceState $StatePath).Halted){throw 'shutdown failure not durable'}
$before=$script:calls.Count
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'late success resumed'}
catch{if($_.Exception.Message -notmatch 'budget_exhausted_or_halted'){throw}}
if($script:calls.Count -ne $before){throw 'late shutdown observed as successful retry'}
if(@($script:calls|Where-Object{$_ -match ':(install|cleanup|launch)$'}).Count){throw 'mutation after shutdown failure'}
}
'1' {
$script:fail='stop'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'missing fault'}
catch{if($_.Exception.Message -notmatch '^injected_'){throw}}
if(-not (Read-CdrMaintenanceState $StatePath).Halted){throw 'shutdown failure not durable'}
$before=$script:calls.Count
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'late success resumed'}
catch{if($_.Exception.Message -notmatch 'budget_exhausted_or_halted'){throw}}
if($script:calls.Count -ne $before){throw 'late shutdown observed as successful retry'}
if(@($script:calls|Where-Object{$_ -match ':(install|cleanup|launch)$'}).Count){throw 'mutation after shutdown failure'}
}
'2' {
$script:fail='bound'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'missing fault'}
catch{if($_.Exception.Message -notmatch '^injected_'){throw}}
if(-not (Read-CdrMaintenanceState $StatePath).Halted){throw 'shutdown failure not durable'}
$before=$script:calls.Count
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'late success resumed'}
catch{if($_.Exception.Message -notmatch 'budget_exhausted_or_halted'){throw}}
if($script:calls.Count -ne $before){throw 'late shutdown observed as successful retry'}
if(@($script:calls|Where-Object{$_ -match ':(install|cleanup|launch)$'}).Count){throw 'mutation after shutdown failure'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
