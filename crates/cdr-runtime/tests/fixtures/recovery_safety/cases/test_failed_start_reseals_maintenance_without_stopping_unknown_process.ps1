param([string]$Variant)
function Start-RustRuntime {$script:newAlive=$true;throw 'controlled startup uncertainty'}
try{Invoke-CdrDeploymentRecovery $statePath;throw 'failed launch accepted'}
catch{if($_.Exception.Message -notmatch 'controlled startup uncertainty'){throw}}
if([IO.File]::ReadAllText($DisablePath) -cne 'owned'){throw 'maintenance seal missing'}
if(Test-Path ($statePath+'.completed')){throw 'false completion receipt'}
try{Invoke-CdrDeploymentRecovery $statePath;throw 'unrecorded replacement accepted'}
catch{if($_.Exception.Message -notmatch 'Unrelated or unrecorded'){throw}}
