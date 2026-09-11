param([string]$Variant)
$s=Get-Content $statePath -Raw|ConvertFrom-Json
$s|Add-Member Version 2
[IO.File]::WriteAllText($statePath,($s|ConvertTo-Json))
[IO.File]::Delete($StopPath)
$script:oldAlive=$true
try{Invoke-CdrDeploymentRecovery $statePath;throw 'v2 adopted'}
catch{if($_.Exception.Message -notmatch 'maintenance_v2'){throw}}
if(Test-Path $StopPath){throw 'stop published without ACK'}
if($script:waits -ne 0 -or $script:starts -ne 0){throw 'legacy process action'}
