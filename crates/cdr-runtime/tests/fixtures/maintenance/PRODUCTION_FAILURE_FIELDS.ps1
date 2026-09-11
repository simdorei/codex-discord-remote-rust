. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'CONNECTED.ps1'), [Text.Encoding]::UTF8)))
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s|Add-Member FailureNoticePhase 'none'
$s|Add-Member FailureReceipt ''
$s|Add-Member FailureNoticeError ''
Save-CdrMaintenanceState $s $StatePath
