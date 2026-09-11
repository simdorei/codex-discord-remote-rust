param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'LOCAL.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
function Assert-CdrCertifiedBaseline {throw 'obsolete certificate gate invoked'}
Invoke-CdrMaintenanceEngine $StatePath $op
if((Read-CdrMaintenanceState $StatePath).Phase -ne 'verified'){throw 'not completed'}
$backup=$script:calls.IndexOf('planned:backup')
$ack=$script:calls.IndexOf('prepared:ACK')
if($backup -lt 0 -or $ack -le $backup){throw 'backup was not before drain'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
