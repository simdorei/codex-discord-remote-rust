param([string]$Variant)
$script:foreign=$true
try{Invoke-CdrDeploymentRecovery $statePath;throw 'foreign process accepted'}
catch{if($_.Exception.Message -notmatch 'Unrelated or unrecorded'){throw}}
if($script:starts -ne 0 -or -not (Test-Path $DisablePath)){throw 'foreign instance affected'}
