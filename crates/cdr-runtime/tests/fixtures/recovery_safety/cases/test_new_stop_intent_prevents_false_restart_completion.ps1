param([string]$Variant)
$script:newAlive=$true
$fence=[pscustomobject]@{RuntimeId='old';ProcessIdentity='42|99';Nonce='owned'}
Write-CdrRestartCompletion $fence
try{Test-CdrRestartCompleted $fence;throw 'active maintenance ignored'}
catch{if($_.Exception.Message -notmatch 'Another maintenance intent'){throw}}
