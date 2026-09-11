param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'LOCAL.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s=Read-CdrMaintenanceState $StatePath;$s.Phase='prepared';Save-CdrMaintenanceState $s $StatePath
$script:fail='backup_bound'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'missing backup accepted'}
catch{if($_.Exception.Message -ne 'injected_backup_bound'){throw}}
if(@($script:calls|Where-Object{$_ -match ':(ACK|stop|install|launch)$'}).Count){throw 'effect without bound backup'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
