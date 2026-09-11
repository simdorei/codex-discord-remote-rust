param([string]$Variant)
[IO.File]::Delete($StopPath);[IO.File]::Delete($DisablePath)
$script:newAlive=$true
$fence=[pscustomobject]@{RuntimeId='old';ProcessIdentity='42|99';Nonce='owned'}
Write-CdrRestartCompletion $fence
if(-not (Test-CdrRestartCompleted $fence)){throw 'valid receipt rejected'}
$fence.Nonce='foreign'
if(Test-CdrRestartCompleted $fence){throw 'foreign receipt accepted'}
