param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'NATIVE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$result=Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','Write-Output probe_ok; exit 0') 5 -PassThru
if($null -eq $result -or $result.ExitCode -ne 0 -or $result.Stdout.Trim() -cne 'probe_ok'){throw 'definite output unavailable'}
if($null -ne (Read-CdrMaintenanceState $StatePath).ActiveCommand){throw 'result before command committed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
