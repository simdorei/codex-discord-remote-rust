param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PREPARED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.CreatedAt=$now.AddMinutes(-40).ToString('o');$s.Deadline=$now.AddMinutes(-10).ToString('o');$s.Halted=$true
Save-CdrMaintenanceState $s $StatePath
$guard=[IO.File]::Open($StatePath,'Open','Read','Read')
try {try{Complete-CdrMaintenance $s $StatePath;throw 'failure missing'}catch{if($_.Exception.Message -eq 'failure missing'){throw}}}
finally{$guard.Dispose()}
if((Test-Path $DisablePath) -or -not (Test-Path $StatePath) -or -not (Test-Path ($StatePath+'.completed'))){throw 'wrong crash boundary'}
Invoke-CdrMaintenanceEngine $StatePath $op
if(Test-Path $StatePath){throw 'expired terminal cleanup stuck'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'reentry replayed side effect'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
