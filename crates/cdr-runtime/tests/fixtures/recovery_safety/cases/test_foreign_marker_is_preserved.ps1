param([string]$Variant)
[IO.File]::WriteAllText($StopPath,'foreign')
try{Invoke-CdrDeploymentRecovery $statePath;throw 'foreign marker accepted'}
catch{if($_.Exception.Message -notmatch 'Foreign maintenance marker'){throw}}
if([IO.File]::ReadAllText($StopPath) -cne 'foreign' -or $script:starts -ne 0){throw 'foreign marker affected'}
