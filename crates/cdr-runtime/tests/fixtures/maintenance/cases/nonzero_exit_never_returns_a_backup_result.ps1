param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'NATIVE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$result=$null
try{$result=Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','Write-Output probe_ok; exit 7') 5 -PassThru;throw 'failure accepted'}
catch{if($_.Exception.Message -notmatch 'native_command_failed exit=7'){throw}}
if($null -ne $result){throw 'failed command result returned'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
