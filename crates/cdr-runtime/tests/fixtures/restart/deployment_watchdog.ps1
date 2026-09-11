param([string]$RepoRoot,[string]$BinaryPath,[string]$RecoverDeploymentStatePath)
$ErrorActionPreference='Stop'
if(-not $RecoverDeploymentStatePath){throw 'generic watchdog prohibited'}
. (Join-Path $env:CDR_SOURCE 'codex-discord-rust-drain.ps1')
. (Join-Path $env:CDR_SOURCE 'codex-discord-rust-control.ps1')
. (Join-Path $env:CDR_SOURCE 'scripts/CdrDeploymentRecovery.ps1')
. (Join-Path $env:CDR_SOURCE 'scripts/CdrLaunchJournal.ps1')
$StopPath=Join-Path $RepoRoot '.codex_discord_rust.stop'
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$DrainPreparePath=Join-Path $RepoRoot '.codex_discord_rust.drain.prepare'
$DrainAckPath=Join-Path $RepoRoot '.codex_discord_rust.drain.ack'
$RestartPath=Join-Path $RepoRoot '.codex_discord_rust.restart'
function Get-Process {param($Id,$ErrorAction) if($Id -eq 77 -and (Test-Path (Join-Path $RepoRoot 'started'))){[pscustomobject]@{Id=77;Path=$BinaryPath}}}
function Get-VerifiedRuntimeProcess {if(Test-Path (Join-Path $RepoRoot 'started')){[pscustomobject]@{Id=77}}}
function Get-RustProcessIdentity {param($Process) if($Process){'77|99'}else{''}}
function Get-VerifiedRuntimeIdentity {Get-RustProcessIdentity (Get-VerifiedRuntimeProcess)}
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
function Clear-DeadRuntimeArtifacts {}
function Start-RustRuntime {
 Set-CdrLaunchStarting
 [IO.File]::WriteAllText((Join-Path $RepoRoot 'started'),'yes')
 Set-CdrLaunchChild ([pscustomobject]@{Id=77;Path=$BinaryPath})
}
function Stop-VerifiedRuntime {throw 'force kill forbidden'}
$control=Enter-CdrControl $RepoRoot
try {Invoke-CdrDeploymentRecovery $RecoverDeploymentStatePath}finally{$control.Dispose()}
