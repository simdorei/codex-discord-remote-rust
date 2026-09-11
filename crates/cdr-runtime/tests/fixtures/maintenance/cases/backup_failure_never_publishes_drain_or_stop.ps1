param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'LOCAL.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$script:fail='backup'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'backup failure not checked'}
catch{if($_.Exception.Message -ne 'injected_backup'){throw}}
if(@($script:calls|Where-Object{$_ -match ':(ACK|stop|install|launch)$'}).Count){throw 'effect after failed backup'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
