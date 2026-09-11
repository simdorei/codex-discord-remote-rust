param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'NATIVE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
try{Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','exit 7') 5;throw 'exit 7 accepted'}
catch{if($_.Exception.Message -notmatch 'native_command_failed exit=7'){throw}}
if($null -ne (Read-CdrMaintenanceState $StatePath).ActiveCommand){throw 'definite exit left unknown'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
