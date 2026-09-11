param([string]$Variant)
$script:newAlive=$true
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$true}}
try{Wait-CdrReplacementReady -ExpectedIdentity '77|99' -TimeoutSeconds 0;throw 'bootstrap accepted'}
catch{if($_.Exception.Message -notmatch 'fresh matching heartbeat'){throw}}
