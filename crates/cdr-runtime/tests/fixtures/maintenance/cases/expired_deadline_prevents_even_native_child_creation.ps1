param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'NATIVE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.Deadline=[DateTimeOffset]::UtcNow.AddSeconds(-1).ToString('o')
try{Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','exit 0');throw 'expired child spawned'}
catch{if($_.Exception.Message -notmatch 'deadline_exceeded'){throw}}
if($null -ne (Read-CdrMaintenanceState $StatePath).ActiveCommand){throw 'expired attempt recorded'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
