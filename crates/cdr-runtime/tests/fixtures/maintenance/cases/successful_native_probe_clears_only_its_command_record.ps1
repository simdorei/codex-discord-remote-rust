param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'NATIVE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','Write-Output probe_ok; exit 0') 5
if($null -ne (Read-CdrMaintenanceState $StatePath).ActiveCommand){throw 'command not completed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
