param([string]$Variant)
$script:oldAlive=$true
function Wait-RustRuntimeExit {throw 'controlled graceful_exit_timeout'}
function Get-HeartbeatHealth {throw 'generic health path was entered'}
try{Invoke-CdrDeploymentRecovery $statePath;throw 'timeout accepted'}
catch{if($_.Exception.Message -notmatch 'controlled graceful_exit_timeout'){throw}}
if($script:starts -ne 0){throw 'started on timeout'}
if([IO.File]::ReadAllText($DisablePath) -cne 'owned' -or [IO.File]::ReadAllText($StopPath) -cne 'owned'){throw 'markers changed'}
