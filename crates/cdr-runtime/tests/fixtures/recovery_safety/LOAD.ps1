$ErrorActionPreference='Stop'
$RepoRoot=$env:V2_ROOT; $BinaryPath=Join-Path $RepoRoot 'fixture.exe'
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-control.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrDeploymentRecovery.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrLaunchJournal.ps1')
$StopPath=Join-Path $RepoRoot '.codex_discord_rust.stop'
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$DrainPreparePath=Join-Path $RepoRoot '.codex_discord_rust.drain.prepare'
$DrainAckPath=Join-Path $RepoRoot '.codex_discord_rust.drain.ack'
$RestartPath=Join-Path $RepoRoot '.codex_discord_rust.restart'
[IO.File]::WriteAllText($BinaryPath,'hash fixture')
$statePath=Join-Path $RepoRoot 'state.json'
$state=[ordered]@{RepoRoot=$RepoRoot;BinaryPath=$BinaryPath;RuntimePid=42;RuntimeTicks='99';Marker='owned';
 BaselineHash='CF3C9430364B43F31062427C0CB1E22EA1E28B34C6A05F5035418D0B8029ACCA';CandidateHash='unused';LogPath=(Join-Path $RepoRoot 'log')}
[IO.File]::WriteAllText($statePath,($state|ConvertTo-Json))
[IO.File]::WriteAllText($StopPath,'owned');[IO.File]::WriteAllText($DisablePath,'owned')
$script:oldAlive=$false;$script:foreign=$false;$script:newAlive=$false;$script:starts=0;$script:waits=0
function Get-Process {
 param($Id,$ErrorAction)
 if($Id -eq 42 -and $script:oldAlive){[pscustomobject]@{Id=42;Path=$BinaryPath;StartTime=[datetime]::new(99,[DateTimeKind]::Utc)}}
 if($Id -eq 77 -and $script:newAlive){[pscustomobject]@{Id=77;Path=$BinaryPath;StartTime=[datetime]::new(99,[DateTimeKind]::Utc)}}
}
function Get-VerifiedRuntimeProcess {
 if($script:foreign){[pscustomobject]@{Id=88}}
 elseif($script:newAlive){[pscustomobject]@{Id=77}}
 elseif($script:oldAlive){[pscustomobject]@{Id=42}}
}
function Get-RustProcessIdentity {param($Process) if($Process){"$($Process.Id)|99"}else{''}}
function Get-VerifiedRuntimeIdentity {Get-RustProcessIdentity (Get-VerifiedRuntimeProcess)}
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
function Clear-DeadRuntimeArtifacts {if($script:oldAlive -or $script:foreign){throw 'live artifacts cleared'}}
function Start-RustRuntime {
 if($script:oldAlive -or $script:foreign){throw 'duplicate start'}
 if((Test-Path $StopPath) -or -not (Test-Path $DisablePath)){throw 'startup must retain maintenance seal without stop'}
 $script:starts++;$script:newAlive=$true
 Set-CdrLaunchStarting
 Set-CdrLaunchChild ([pscustomobject]@{Id=77;Path=$BinaryPath})
}
function Stop-VerifiedRuntime {throw 'KILL MUST NEVER BE CALLED'}
function Wait-RustRuntimeExit {$script:waits++;$script:oldAlive=$false}
