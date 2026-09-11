param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PREPARED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.RuntimeEvidence.Heartbeats=@();Save-CdrMaintenanceState $s $StatePath
try{Complete-CdrMaintenance $s $StatePath;throw 'FAULT_NOT_REJECTED'}
catch{if($_.Exception.Message -notmatch 'maintenance_evidence_heartbeat'){throw}}
if(-not (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'corrupt proof unsealed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
