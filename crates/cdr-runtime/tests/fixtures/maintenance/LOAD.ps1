$ErrorActionPreference='Stop'
$RepoRoot=$env:V2_ROOT; $BinaryPath=Join-Path $RepoRoot 'runtime.exe'
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-control.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceState.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceEngine.ps1')
$StatePath=Get-CdrMaintenancePath $RepoRoot
$op='1234567890abcdef1234567890abcdef'
$bundle=Join-Path $RepoRoot ('.codex-discord-backups/maintenance-v2-'+$op)
[void][IO.Directory]::CreateDirectory($bundle)
$now=[DateTimeOffset]::UtcNow
$state=[pscustomobject]@{
 Version=2;ShutdownPolicy='live-handshake-v1';Operation=$op;RepoRoot=$RepoRoot;BinaryPath=$BinaryPath;Phase='planned'
 Attempts=0;Halted=$false;LastError='';Bundle=$bundle;CreatedAt=$now.ToString('o');Deadline=$now.AddMinutes(30).ToString('o')
 Fence=[pscustomobject]@{RuntimeId='runtime1';ProcessIdentity='42|99';Nonce=$op}
 BaselineHash=('A'*64);CandidateHash=('B'*64);OperatorHash=('C'*64);EnvHash=('D'*64)
 CandidatePath=(Join-Path $bundle 'candidate.exe');OperatorPath=(Join-Path $bundle 'operator.exe')
 TaskName=('Codex Maintenance V2 '+$op);NotifyChannel='900000000000000001';DiscordReceipt='';Heartbeats=@()
 ActiveCommand=$null;PreStopBackup=$null;PostStopBackup=$null;FailureObservation=$null
}
Write-NewCdrMarker $StatePath ($state|ConvertTo-Json -Depth 10)
$script:calls=[Collections.Generic.List[string]]::new()
$script:fail='';$script:crashed=$false
function Effect([string]$Name) {
 $phase=(Read-CdrMaintenanceState $StatePath).Phase
 $script:calls.Add("${phase}:$Name")
 if($script:fail -eq $Name -and -not $script:crashed){$script:crashed=$true;throw "injected_$Name"}
}
function Assert-CdrMaintenanceArtifacts {Effect 'artifacts'}
function Assert-CdrMaintenanceMarkers {Effect 'markers'}
function Assert-CdrCertifiedBaseline {Effect 'T1'}
function Invoke-CdrMaintenancePreStopBackup {Effect 'backup'}
function Assert-CdrMaintenancePreStopBackup {Effect 'backup_bound'}
function Assert-CdrMaintenanceBackupReceipt {Effect 'backup_receipt'}
function Get-CdrMaintenanceFailureObservation {[pscustomobject]@{Original='unknown';Ack='unknown';Stop='unknown';Code='fixture_failure';ObservedAt='fixture'}}
function Assert-CdrMaintenanceRecoveryArmed {Effect 'armed'}
function Invoke-CdrMaintenancePreflight {Effect 'preflight'}
function Invoke-CdrMaintenanceDrain {Effect 'ACK'}
function Assert-RestartDrainBound {Effect 'bound'}
function Invoke-CdrMaintenanceStop {Effect 'stop'}
function Assert-CdrMaintenanceNoRuntime {Effect 'old_gone'}
function Invoke-CdrMaintenancePackaging {Effect 'package'}
function Install-CdrMaintenanceCandidate {Effect 'install'}
function Invoke-CdrMaintenanceCleanup {
 if((Read-CdrMaintenanceState $StatePath).Phase -ne 'mutation_started'){throw 'mutation intent not durable'}
 Effect 'cleanup'
}
function Invoke-CdrMaintenanceFullReadiness {Effect 'full_readiness'}
function Invoke-CdrMaintenanceLaunch {Effect 'launch'}
function Wait-CdrMaintenanceHeartbeats {param($s,$p);Effect 'heartbeats';$s.Heartbeats=@(1,2)}
function Send-CdrMaintenanceResult {Effect 'notify';return '12345'}
function Complete-CdrMaintenance {Effect 'complete'}
function Publish-CdrMaintenanceFailure {}
function Stop-VerifiedRuntime {throw 'FORCE KILL FORBIDDEN'}
function Invoke-CdrDeploymentRecovery {throw 'LEGACY RECOVERY FORBIDDEN'}
