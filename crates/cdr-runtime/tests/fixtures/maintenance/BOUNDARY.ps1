. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceActions.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceLaunch.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrLaunchJournal.ps1')
$StopPath=Join-Path $RepoRoot '.codex_discord_rust.stop'
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$DrainPreparePath=Join-Path $RepoRoot '.codex_discord_rust.drain.prepare'
$DrainAckPath=Join-Path $RepoRoot '.codex_discord_rust.drain.ack'
$DrainIdentityPath=Join-Path $RepoRoot '.codex_discord_rust.drain.identity'
$HeartbeatPath=Join-Path $RepoRoot '.codex_discord_rust.heartbeat'
$RestartPath=Join-Path $RepoRoot '.codex_discord_rust.restart'
$s=Read-CdrMaintenanceState $StatePath
$s.Phase='launch_ready';Save-CdrMaintenanceState $s $StatePath
[IO.File]::WriteAllText($DisablePath,$s.Operation)
$script:starts=0;$script:childAlive=$false
function Get-RustProcessIdentity {param($Process) if($Process){"$($Process.Id)|99"}else{''}}
function Get-Process {param($Id,$Name,$ErrorAction)
 if($script:childAlive -and ($Id -eq 77 -or $Name -eq 'cdr-runtime')){[pscustomobject]@{Id=77;Path=$BinaryPath}}
}
function Get-VerifiedRuntimeIdentity {if($script:childAlive){'77|99'}else{''}}
function Get-RuntimePid {0}
function Clear-DeadRuntimeArtifacts {}
function Invoke-CdrMaintenanceFullReadiness {Effect 'full_readiness'}
function Assert-CdrMaintenanceArtifacts {Effect 'artifacts'}
function Start-RustRuntime {
 $script:starts++;Set-CdrLaunchStarting;$script:childAlive=$true
 Set-CdrLaunchChild ([pscustomobject]@{Id=77;Path=$BinaryPath})
}
