[CmdletBinding()]
param(
    [string]$RepoRoot,
    [string]$OutputDirectory,
    [string]$EvidenceOutputDirectory,
    [string]$DatabaseSnapshotPath,
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[A-Fa-f0-9]{64}$')]
    [string]$ExpectedRuntimeSha256,
    [ValidatePattern('^[A-Fa-f0-9]{64}$')]
    [string]$ExpectedDatabaseSha256 = 'BD1A83372D5FE95BBE27D76FE344826098CD761F02171187905E8CA9505C6813',
    [Parameter(Mandatory = $true)]
    [string]$OfflineSoakEvidencePath,
    [Parameter(Mandatory = $true)]
    [string]$WorkspaceGateEvidencePath,
    [ValidatePattern('^\d{8}T\d{9}Z$')]
    [string]$CheckpointStamp,
    [switch]$AllowExternalOutput,
    [ValidateRange(1, 30)]
    [int]$OfflineSmokeDurationSeconds = 1
)

$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false, $true)
foreach ($helper in @(
    'RustMigrationCheckpoint.Package.psm1',
    'RustMigrationCheckpoint.Verify.psm1',
    'RustMigrationCheckpoint.Common.psm1', 'RustMigrationCheckpoint.Evidence.psm1',
    'RustMigrationCheckpoint.ProcessSafety.psm1'
)) { Import-Module (Join-Path $PSScriptRoot $helper) -ErrorAction Stop }

$ExpectedRuntimeSha256 = $ExpectedRuntimeSha256.ToUpperInvariant()
$ExpectedDatabaseSha256 = $ExpectedDatabaseSha256.ToUpperInvariant()
if ([string]::IsNullOrWhiteSpace($RepoRoot)) { $RepoRoot = Split-Path -Parent $PSScriptRoot }
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
$repositoryBackupRoot = Join-Path $RepoRoot '.codex-discord-backups'
$repositoryEvidenceRoot = Join-Path $RepoRoot 'docs\rust-migration\evidence'
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = $repositoryBackupRoot
} else { $OutputDirectory = Resolve-CdrCheckpointPath $RepoRoot $OutputDirectory }
if ([string]::IsNullOrWhiteSpace($EvidenceOutputDirectory)) {
    $EvidenceOutputDirectory = $repositoryEvidenceRoot
} else { $EvidenceOutputDirectory = Resolve-CdrCheckpointPath $RepoRoot $EvidenceOutputDirectory }
if ([string]::IsNullOrWhiteSpace($DatabaseSnapshotPath)) {
    $DatabaseSnapshotPath = Join-Path $RepoRoot (
        '.codex-discord-backups\discord_mirror.v2-cutover.' +
        '20260831T082055Z.7ceb8252f094.sqlite'
    )
} else { $DatabaseSnapshotPath = Resolve-CdrCheckpointPath $RepoRoot $DatabaseSnapshotPath }
if (-not $AllowExternalOutput) {
    $OutputDirectory = Assert-CdrPathUnderRoot `
        $OutputDirectory $repositoryBackupRoot 'OutputDirectory'
    $EvidenceOutputDirectory = Assert-CdrPathUnderRoot `
        $EvidenceOutputDirectory $repositoryEvidenceRoot 'EvidenceOutputDirectory'
}
$DatabaseSnapshotPath = Assert-CdrPathUnderRoot `
    $DatabaseSnapshotPath $repositoryBackupRoot 'DatabaseSnapshotPath'
Assert-CdrCheckpointSource $DatabaseSnapshotPath $RepoRoot

$stamp = if ([string]::IsNullOrWhiteSpace($CheckpointStamp)) {
    [datetime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')
} else { $CheckpointStamp }
$archivePath = Join-Path $OutputDirectory "cdr-rust-local-release-checkpoint-$stamp.zip"
$evidencePath = Join-Path $EvidenceOutputDirectory "local-release-checkpoint-$stamp.json"
if (-not $AllowExternalOutput) {
    $archivePath = Assert-CdrPathUnderRoot $archivePath $repositoryBackupRoot 'ArchivePath'
    $evidencePath = Assert-CdrPathUnderRoot $evidencePath $repositoryEvidenceRoot 'EvidencePath'
}
$disablePath = Join-Path $RepoRoot '.codex_discord_bot.disabled'
$stagingRoot = $null; $verificationRoot = $null; $markerGuard = $null; $archiveGuard = $null
$archiveCreated = $false; $evidenceCreated = $false

try {
    foreach ($directory in @(
        [pscustomobject]@{
            Path = $repositoryBackupRoot; Root = $RepoRoot; Label = 'Repository checkpoint root'
            OverrideEligible = $false
        },
        [pscustomobject]@{
            Path = $repositoryEvidenceRoot; Root = $RepoRoot; Label = 'Repository evidence root'
            OverrideEligible = $false
        },
        [pscustomobject]@{
            Path = $OutputDirectory; Root = $repositoryBackupRoot; Label = 'OutputDirectory'
            OverrideEligible = $true
        },
        [pscustomobject]@{
            Path = $EvidenceOutputDirectory; Root = $repositoryEvidenceRoot
            Label = 'EvidenceOutputDirectory'; OverrideEligible = $true
        }
    )) {
        if ($AllowExternalOutput -and $directory.OverrideEligible) {
            $null = Assert-CdrSafeAbsoluteDirectoryPath $directory.Path $directory.Label
        } else {
            $null = Assert-CdrSafeDirectoryPath $directory.Path $directory.Root $directory.Label
        }
        $null = New-Item -ItemType Directory -Path $directory.Path -Force
        if ($AllowExternalOutput -and $directory.OverrideEligible) {
            $null = Assert-CdrSafeAbsoluteDirectoryPath $directory.Path $directory.Label
        } else {
            $null = Assert-CdrSafeDirectoryPath $directory.Path $directory.Root $directory.Label
        }
    }
    $evidenceBundle = Get-CdrCheckpointEvidenceBundle `
        -RepoRoot $RepoRoot `
        -RepositoryEvidenceRoot $repositoryEvidenceRoot `
        -OfflineSoakEvidencePath $OfflineSoakEvidencePath `
        -WorkspaceGateEvidencePath $WorkspaceGateEvidencePath `
        -ExpectedRuntimeSha256 $ExpectedRuntimeSha256
    if (Test-Path -LiteralPath $archivePath) { throw "Checkpoint archive already exists: $archivePath" }
    if (Test-Path -LiteralPath $evidencePath) { throw "Checkpoint evidence already exists: $evidencePath" }
    $markerBefore = Assert-DisabledMarker $disablePath $null
    $markerGuard = [IO.File]::Open(
        $disablePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read
    )
    $processesBefore = @(Get-CdrCheckpointForbiddenProcessSnapshot -RepoRoot $RepoRoot)
    Assert-CdrCheckpointBotOff $processesBefore 'before_packaging'
    $stagingRoot = New-CdrCheckpointTempDirectory 'stage'
    $package = New-CdrCheckpointPackage `
        -RepoRoot $RepoRoot `
        -StagingRoot $stagingRoot `
        -ArchivePath $archivePath `
        -DatabaseSnapshotPath $DatabaseSnapshotPath `
        -ExpectedRuntimeSha256 $ExpectedRuntimeSha256 `
        -ExpectedDatabaseSha256 $ExpectedDatabaseSha256 `
        -OfflineSoakSnapshot $evidenceBundle.OfflineSoakSnapshot `
        -WorkspaceGateSnapshot $evidenceBundle.WorkspaceGateSnapshot `
        -SourceFingerprint $evidenceBundle.SourceFingerprint `
        -RollbackSource $evidenceBundle.RollbackSource `
        -ExternalOutputOverride ([bool]$AllowExternalOutput)
    $archiveCreated = $true
    $archiveGuard = [IO.File]::Open(
        $archivePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read
    )
    $verificationRoot = New-CdrCheckpointTempDirectory 'verify'
    $verification = Test-CdrCheckpointArchive `
        -ArchivePath $archivePath -ArchiveStream $archiveGuard `
        -VerificationRoot $verificationRoot `
        -PayloadFiles $package.PayloadFiles -ControlFiles $package.ControlFiles `
        -ExpectedMetadata $package.Metadata `
        -OfflineSmokeDurationSeconds $OfflineSmokeDurationSeconds `
        -ForbiddenProcessesBefore $processesBefore `
        -DisablePath $disablePath `
        -MarkerBefore $markerBefore
    $archiveHash = $verification.ArchiveHash
    $evidence = [ordered]@{
        schema_version = 1
        kind = 'cdr_rust_migration_local_release_checkpoint_verification'
        created_at_utc = [datetime]::UtcNow.ToString('o')
        status = 'passed'
        checkpoint_stage = 'final_source_local_checkpoint'
        local_archive_verified = $true
        immutable_release_checkpoint_still_pending_authorization = $true
        conclusion = 'Local archive verified; immutable release checkpoint still pending authorization.'
        binary_provenance = (
            "The packaged cdr-runtime.exe is exactly SHA-256 $($package.RuntimeHash). " +
            "Its same-run offline proof matches Rust source $($package.SourceFingerprint)."
        )
        archive = [ordered]@{
            path = $archivePath
            sha256 = $archiveHash
            bytes = $verification.ArchiveBytes
            metadata_sha256 = Get-CdrSha256 $verification.MetadataPath
            sha256sums_sha256 = Get-CdrSha256 $verification.SumsPath
            payload_file_count = @($verification.Metadata.payload_files).Count
        }
        verified_inputs = [ordered]@{
            cdr_runtime_sha256 = $package.RuntimeHash
            rollback_database_sha256 = $package.DatabaseHash
            rollback_database_bytes = $package.DatabaseBytes
            offline_soak_evidence_sha256 = $package.OfflineSoakEvidenceHash
            workspace_gate_evidence_sha256 = $package.WorkspaceGateEvidenceHash
            source_fingerprint = $package.SourceFingerprint
            powershell_source_file_count = $evidenceBundle.PowerShellSource.file_count
            rollback_source_file_count = $package.RollbackSourceFileCount
        }
        verification = [ordered]@{
            fresh_extraction = $true
            exact_names_verified = $true
            all_payload_hashes_verified = $true
            pe_magic_verified = @('cdr-runtime.exe', 'cdr-offline-soak.exe', 'cdr-mcp-server.exe', 'cdr-pro-helper.exe')
            source_rollback = [ordered]@{
                status = $verification.RollbackVerification.Status
                scope = 'local_windows_bot_and_operational_tooling'
                powershell_watchdog_dry_run = $verification.RollbackVerification.PowerShellWatchdogDryRun
                powershell_source_parse = $verification.RollbackVerification.PowerShellSourceParse
                powershell_source_file_count = $verification.RollbackVerification.PowerShellSourceFileCount
                memory_ab_module_import_preflight = $verification.RollbackVerification.MemoryAbModuleImportPreflight
                rust_setup_preflight = $verification.RollbackVerification.RustSetupPreflight
                rust_pro_helper_preflight = $verification.RollbackVerification.RustProHelperPreflight
                source_file_count = $verification.RollbackVerification.SourceFileCount
                requires_python = $verification.RollbackVerification.RequiresPython
            }
            forbidden_entry_count = 0
            offline_smoke = [ordered]@{
                executable = 'cdr-offline-soak.exe'
                duration_seconds = $OfflineSmokeDurationSeconds
                mode = $verification.SmokeSummary.mode
                status = $verification.SmokeSummary.status
                cycles = $verification.SmokeSummary.cycles
                event_line_count = $verification.EventCount
                summary_sha256 = Get-CdrSha256 $verification.SmokeSummaryPath
                events_sha256 = Get-CdrSha256 $verification.SmokeEventsPath
            }
        }
        safety = [ordered]@{
            bot_disabled = $true
            disabled_marker_sha256_before = $markerBefore.Sha256
            disabled_marker_sha256_after = $verification.MarkerAfter.Sha256
            disabled_marker_preserved = $true
            cdr_runtime_admin_only_executed = $true
            cdr_mcp_server_executed = $false
            bot_or_network_started = $false
            local_checkpoint_only = $true
            external_output_override = [bool]$AllowExternalOutput
            secrets_in_evidence = $false
        }
    }
    $evidenceBytes = $utf8.GetBytes(($evidence | ConvertTo-Json -Depth 30))
    $evidenceStream = [IO.File]::Open(
        $evidencePath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None
    )
    $evidenceCreated = $true
    try { $evidenceStream.Write($evidenceBytes, 0, $evidenceBytes.Length) }
    finally { $evidenceStream.Dispose() }
    Write-Output ([ordered]@{
        archive_path = $archivePath
        archive_sha256 = $archiveHash
        evidence_path = $evidencePath
        status = 'passed'
        local_checkpoint_only = $true
    } | ConvertTo-Json -Compress)
} catch {
    if ($null -ne $archiveGuard) { $archiveGuard.Dispose(); $archiveGuard = $null }
    if ($evidenceCreated -and (Test-Path -LiteralPath $evidencePath -PathType Leaf)) {
        Remove-Item -LiteralPath $evidencePath -Force
    }
    if ($archiveCreated -and (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        Remove-Item -LiteralPath $archivePath -Force
    }
    throw
} finally {
    if ($null -ne $archiveGuard) { $archiveGuard.Dispose() }
    if ($null -ne $markerGuard) { $markerGuard.Dispose() }
    Remove-CdrCheckpointTempDirectory $verificationRoot
    Remove-CdrCheckpointTempDirectory $stagingRoot
}
