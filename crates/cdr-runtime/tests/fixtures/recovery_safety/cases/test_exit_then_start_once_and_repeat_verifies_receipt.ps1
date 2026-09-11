param([string]$Variant)
$script:oldAlive=$true
Invoke-CdrDeploymentRecovery $statePath
Invoke-CdrDeploymentRecovery $statePath
if($script:waits -ne 1 -or $script:starts -ne 1){throw 'wrong handoff count'}
if(-not (Test-Path ($statePath+'.completed'))){throw 'missing completion receipt'}
