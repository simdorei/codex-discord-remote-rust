. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceCommand.ps1')
$s=Read-CdrMaintenanceState $StatePath
$shell=(Get-Command powershell.exe).Source
