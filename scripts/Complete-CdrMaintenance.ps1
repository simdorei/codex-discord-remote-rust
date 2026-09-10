param(
    [Parameter(Mandatory=$true)][string]$ManifestPath,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-F0-9]{64}$')][string]$ManifestHash,
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{32}$')][string]$ExpectedOperation,
    [switch]$Apply
)
$ErrorActionPreference='Stop'
function Get-CdrReviewedFileHash([string]$Path) {
    $sha=[Security.Cryptography.SHA256]::Create();$stream=[IO.File]::OpenRead($Path)
    try{[BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-','')}
    finally{$stream.Dispose();$sha.Dispose()}
}
$RepoRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$expectedManifest=Join-Path $RepoRoot ('.codex-discord-backups/maintenance-v2-'+$ExpectedOperation+'/completion-review.json')
if([IO.Path]::GetFullPath($ManifestPath) -cne $expectedManifest){throw 'reconcile_manifest_path_not_owned'}
if((Get-CdrReviewedFileHash $ManifestPath) -cne $ManifestHash){throw 'reconcile_manifest_hash_mismatch'}
$manifest=Get-Content -LiteralPath $ManifestPath -Raw -Encoding UTF8|ConvertFrom-Json
if($manifest.Operation -cne $ExpectedOperation -or $manifest.RepoRoot -cne $RepoRoot){throw 'reconcile_expected_operation_mismatch'}
# Verify every module before loading any. The manifest digest is an explicit
# caller-supplied reviewed artifact, not a rewrite of the historical program pins.
$modules=@('codex-discord-rust-control.ps1','codex-discord-rust-drain.ps1','scripts/CdrLaunchJournal.ps1',
    'scripts/CdrMaintenanceState.ps1','scripts/CdrMaintenanceBackup.ps1','scripts/CdrMaintenanceCompletionAudit.ps1',
    'scripts/CdrMaintenanceCompletion.ps1','scripts/CdrMaintenanceNotificationResult.ps1',
    'scripts/CdrMaintenanceNoticeJournal.ps1','scripts/CdrMaintenanceReconcileManifest.ps1',
    'scripts/CdrMaintenanceObservation.ps1','scripts/CdrMaintenanceReconcile.ps1')
foreach($path in @($modules+'scripts/Complete-CdrMaintenance.ps1')){
    $pins=@($manifest.SourcePins|Where-Object {$_.Path -ceq $path})
    if($pins.Count -ne 1 -or (Get-CdrReviewedFileHash (Join-Path $RepoRoot $path)) -cne $pins[0].Hash){throw 'reconcile_reviewed_entry_source_changed'}
}
foreach($path in $modules){. (Join-Path $RepoRoot $path)}
$BinaryPath=Join-Path $RepoRoot 'target\release\cdr-runtime.exe'
$EnvPath=Join-Path $RepoRoot '.env'
$StatePath=Get-CdrMaintenancePath $RepoRoot
$LockPath=Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'
$HeartbeatPath=Join-Path $RepoRoot '.codex_discord_rust.heartbeat'
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$StopPath=Join-Path $RepoRoot '.codex_discord_rust.stop'
$DrainPreparePath=Join-Path $RepoRoot '.codex_discord_rust.drain.prepare'
$DrainAckPath=Join-Path $RepoRoot '.codex_discord_rust.drain.ack'
$RestartPath=Join-Path $RepoRoot '.codex_discord_rust.restart'
$operationGuard=$null;$controlGuard=$null
try {
    $operationGuard=[IO.File]::Open(($StatePath+'.lock'),'OpenOrCreate','ReadWrite','None')
    $controlGuard=Enter-CdrControl $RepoRoot -MaintenanceV2
    if((Get-CdrArtifactHash $ManifestPath) -cne $ManifestHash){throw 'reconcile_manifest_changed_before_lock'}
    Invoke-CdrCompletionReconcile $manifest $ManifestHash $StatePath -Apply:$Apply
} finally {if($controlGuard){$controlGuard.Dispose()};if($operationGuard){$operationGuard.Dispose()}}
