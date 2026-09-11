. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'CONNECTED.ps1'),[Text.Encoding]::UTF8)))
Wait-CdrMaintenanceHeartbeats $s $StatePath
$s|Add-Member RuntimeEvidence (Get-CdrRuntimeCompletionEvidence $s)
$null=Initialize-CdrMaintenanceNotice $s
$s.Phase='verified';Save-CdrMaintenanceState $s $StatePath
