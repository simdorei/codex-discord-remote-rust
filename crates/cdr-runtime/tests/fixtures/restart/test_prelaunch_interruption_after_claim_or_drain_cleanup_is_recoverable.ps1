$fence=Get-RestartDrainFence $RestartPath
$journalPath=Join-Path $root '.codex_discord_rust.restart.launch'
$record=New-CdrLaunchJournal $journalPath ('restart:'+$fence.Nonce) $fence
$record.ClaimPath=$RestartClaimPath
Save-CdrLaunchJournal $journalPath $record
[IO.File]::Move($RestartPath,$RestartClaimPath)
[IO.File]::Delete($DrainPreparePath)
# Simulate a later entry after a crash with partial cleanup. No in-memory receipt.
& $entry
if($script:starts -ne 1){throw 'recovery did not start exactly once'}
if(Test-Path $RestartClaimPath){throw 'claim leaked'}
