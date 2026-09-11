param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'NATIVE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
try{Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','Start-Sleep -Seconds 2; exit 0') 1;throw 'wait unbounded'}
catch{if($_.Exception.Message -notmatch 'command_deadline_outcome_unknown'){throw}}
$saved=Read-CdrMaintenanceState $StatePath
if($saved.ActiveCommand.Phase -ne 'child'){throw 'child identity not durable'}
$childPid=[int]$saved.ActiveCommand.Identity.Split('|')[0]
$child=Get-Process -Id $childPid -ErrorAction SilentlyContinue
if($child){[void]$child.WaitForExit(5000)} # Only this harmless fixture exits itself; no kill.
try{Invoke-CdrMaintenanceCommand $saved $shell @('-NoProfile','-Command','exit 0') 1;throw 'command replayed'}
catch{if($_.Exception.Message -notmatch 'command_outcome_unknown'){throw}}
}
default { throw "Unknown native fixture variant: $Variant" }
}
