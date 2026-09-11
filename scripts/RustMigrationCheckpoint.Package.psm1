Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Payload.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Stage.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.ArchivePublish.psm1') -ErrorAction Stop
$ErrorActionPreference = 'Stop'

function New-CdrCheckpointPackage {
    [CmdletBinding()]
    param(
        [string]$RepoRoot,
        [string]$StagingRoot,
        [string]$ArchivePath,
        [string]$DatabaseSnapshotPath,
        [string]$ExpectedRuntimeSha256,
        [string]$ExpectedDatabaseSha256,
        [object]$OfflineSoakSnapshot,
        [object]$WorkspaceGateSnapshot,
        [object]$SourceFingerprint,
        [object]$RollbackSource,
        [bool]$ExternalOutputOverride = $false
    )
    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    $payload = [Collections.Generic.List[object]]::new()
    $OfflineSoakEvidencePath = $OfflineSoakSnapshot.Path
    $OfflineSoakEvidenceHash = $OfflineSoakSnapshot.Hash; $OfflineSoak = $OfflineSoakSnapshot.Value
    $WorkspaceGateEvidencePath = $WorkspaceGateSnapshot.Path
    $WorkspaceGateEvidenceHash = $WorkspaceGateSnapshot.Hash; $WorkspaceGate = $WorkspaceGateSnapshot.Value

    function Add-PayloadFile {
        param(
            [string]$Source, [string]$ArchiveRelativePath,
            [string]$ExpectedSha256, [long]$ExpectedBytes = -1,
            [string]$ExpectedLabel = 'Checkpoint payload'
        )
        $copy = @{
            Source = $Source; RepoRoot = $RepoRoot; StagingRoot = $StagingRoot
            ArchiveRelativePath = $ArchiveRelativePath
            ExpectedSha256 = $ExpectedSha256; ExpectedBytes = $ExpectedBytes
            ExpectedLabel = $ExpectedLabel
        }
        $record = Copy-CdrCheckpointPayloadFile @copy
        $null = $payload.Add($record)
        return $record
    }

    $artifactSources = [ordered]@{
        'cdr-runtime.exe' = Join-Path $RepoRoot 'target\release\cdr-runtime.exe'
        'cdr-offline-soak.exe' = Join-Path $RepoRoot 'target\release\cdr-offline-soak.exe'
        'cdr-mcp-server.exe' = Join-Path $RepoRoot 'target\release\cdr-mcp-server.exe'
        'cdr-pro-helper.exe' = Join-Path $RepoRoot 'target\release\cdr-pro-helper.exe'
    }
    $evidenceNames = [ordered]@{
        'cdr-runtime.exe' = 'cdr_runtime'
        'cdr-offline-soak.exe' = 'cdr_offline_soak'
        'cdr-mcp-server.exe' = 'cdr_mcp_server'
        'cdr-pro-helper.exe' = 'cdr_pro_helper'
    }
    $artifactPayload = [ordered]@{}
    foreach ($name in $evidenceNames.Keys) {
        $evidenceName = $evidenceNames[$name]
        $soakArtifact = $OfflineSoak.artifacts.$evidenceName
        $gateArtifact = $WorkspaceGate.artifacts.$evidenceName
        if ($soakArtifact.sha256 -cne $gateArtifact.sha256 -or
            $soakArtifact.bytes -ne $gateArtifact.bytes) {
            throw "$name evidence records disagree before staging."
        }
        $record = Add-PayloadFile $artifactSources[$name] "artifacts/$name" `
            $soakArtifact.sha256 ([long]$soakArtifact.bytes) "$name evidence-bound payload"
        $artifactPayload[$name] = $record
        Assert-PeMagic (Join-Path $StagingRoot $record.path.Replace('/', '\'))
    }
    $runtimeHash = $artifactPayload['cdr-runtime.exe'].sha256
    if ($runtimeHash -cne $ExpectedRuntimeSha256) {
        throw "cdr-runtime.exe SHA-256 mismatch: expected=$ExpectedRuntimeSha256 actual=$runtimeHash"
    }

    $rollbackEntries = @(Get-CdrCheckpointRollbackSourceEntries -RepoRoot $RepoRoot)
    if ($null -eq $RollbackSource -or $rollbackEntries.Count -ne $RollbackSource.file_count) {
        throw 'Rollback source inventory no longer matches the verified workspace evidence.'
    }
    $rollbackRows = @($RollbackSource.files)
    for ($index = 0; $index -lt $rollbackEntries.Count; $index++) {
        $entry = $rollbackEntries[$index]
        $verified = $rollbackRows[$index]
        if ($entry.source_path -cne $verified.path -or
            $entry.archive_path -cne $verified.archive_path) {
            throw "Rollback source path changed after evidence verification at index $index."
        }
        $null = Add-PayloadFile $entry.source $entry.archive_path $verified.sha256 `
            ([long]$verified.bytes) "Rollback source $($verified.path)"
    }
    $workspacePayload = [ordered]@{}
    $sourceRows = [Collections.Generic.Dictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($row in $SourceFingerprint.files) { $sourceRows.Add([string]$row.path, $row) }
    foreach ($relative in @(Get-CdrCheckpointWorkspacePaths)) {
        $source = Join-Path $RepoRoot $relative
        $verified = $null
        if (-not $sourceRows.TryGetValue($relative, [ref]$verified)) {
            throw "Workspace payload is absent from the verified source fingerprint: $relative"
        }
        $workspacePayload[$relative] = Add-PayloadFile `
            $source "workspace/$relative" $verified.sha256 ([long]$verified.bytes) `
            "Workspace payload $relative"
    }

    $databaseName = [IO.Path]::GetFileName($DatabaseSnapshotPath)
    $databasePayload = Add-PayloadFile $DatabaseSnapshotPath "rollback/$databaseName" `
        $ExpectedDatabaseSha256 -1 'Rollback DB'
    $databaseHash = $databasePayload.sha256
    $offlineEvidencePayload = Add-PayloadFile $OfflineSoakEvidencePath `
        "evidence/$([IO.Path]::GetFileName($OfflineSoakEvidencePath))" `
        $OfflineSoakEvidenceHash $OfflineSoakSnapshot.Bytes 'Offline soak evidence'
    $workspaceEvidencePayload = Add-PayloadFile $WorkspaceGateEvidencePath `
        "evidence/$([IO.Path]::GetFileName($WorkspaceGateEvidencePath))" `
        $WorkspaceGateEvidenceHash $WorkspaceGateSnapshot.Bytes 'Workspace gate evidence'

    $gitCheck = Invoke-CdrNativeCapture 'git' @('-C', $RepoRoot, 'rev-parse', '--is-inside-work-tree')
    if ($gitCheck.ExitCode -ne 0 -or $gitCheck.Text -ne 'true') {
        throw "Repository Git metadata could not be read: $($gitCheck.Text)"
    }
    $gitStatus = Invoke-CdrNativeCapture 'git' @(
        '-C', $RepoRoot, 'status', '--porcelain=v1', '--untracked-files=all'
    )
    if ($gitStatus.ExitCode -ne 0) { throw "Git status failed: $($gitStatus.Text)" }
    $statusLines = @($gitStatus.Lines | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $gitHead = Invoke-CdrNativeCapture 'git' @('-C', $RepoRoot, 'rev-parse', 'HEAD')
    $gitBranch = Invoke-CdrNativeCapture 'git' @('-C', $RepoRoot, 'branch', '--show-current')
    $rustc = Invoke-CdrNativeCapture 'rustc' @('-Vv')
    $cargo = Invoke-CdrNativeCapture 'cargo' @('-V')
    if ($rustc.ExitCode -ne 0) { throw "rustc toolchain query failed: $($rustc.Text)" }
    if ($cargo.ExitCode -ne 0) { throw "cargo toolchain query failed: $($cargo.Text)" }
    $rustupCommand = Get-Command rustup -ErrorAction SilentlyContinue
    $rustup = if ($null -eq $rustupCommand) { $null } else {
        Invoke-CdrNativeCapture $rustupCommand.Source @('show', 'active-toolchain')
    }

    $ordinalPayload = [Collections.Generic.SortedDictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($record in $payload) { $ordinalPayload.Add([string]$record.path, $record) }
    $sortedPayload = [object[]]@($ordinalPayload.Values)
    $artifactMetadata = [ordered]@{}
    foreach ($name in $artifactSources.Keys) {
        $record = $artifactPayload[$name]
        $artifactMetadata[$name] = [ordered]@{
            sha256 = $record.sha256
            bytes = $record.bytes
            archive_path = "artifacts/$name"
        }
    }
    $metadata = [ordered]@{
        schema_version = 1
        kind = 'cdr_rust_migration_local_release_checkpoint'
        created_at_utc = [datetime]::UtcNow.ToString('o')
        checkpoint_stage = 'final_source_local_checkpoint'
        binary_provenance = (
            "The packaged cdr-runtime.exe is exactly SHA-256 $runtimeHash. " +
            "The same-run short offline proof binds it to Rust source fingerprint " +
            "$($SourceFingerprint.aggregate_sha256)."
        )
        repo = $RepoRoot
        worktree_uncommitted_state = [ordered]@{
            is_dirty = $statusLines.Count -gt 0
            status_porcelain_v1 = @($statusLines)
            head = $(if ($gitHead.ExitCode -eq 0) { $gitHead.Text } else { $null })
            branch = $(if ($gitBranch.ExitCode -eq 0) { $gitBranch.Text } else { $null })
        }
        toolchain = [ordered]@{
            rustc_verbose = $rustc.Text
            cargo = $cargo.Text
            active_rustup_toolchain = $(
                if ($null -ne $rustup -and $rustup.ExitCode -eq 0) { $rustup.Text } else { $null }
            )
            rust_toolchain_toml_sha256 = $workspacePayload['rust-toolchain.toml'].sha256
        }
        current_artifact_hashes = $artifactMetadata
        source_fingerprint = [ordered]@{
            schema = $SourceFingerprint.schema
            aggregate_sha256 = $SourceFingerprint.aggregate_sha256
            file_count = $SourceFingerprint.file_count
            total_bytes = $SourceFingerprint.total_bytes
        }
        offline_soak_evidence = [ordered]@{
            filename = [IO.Path]::GetFileName($OfflineSoakEvidencePath)
            sha256 = $offlineEvidencePayload.sha256
            status = $OfflineSoak.status
        }
        workspace_gate_evidence = [ordered]@{
            filename = [IO.Path]::GetFileName($WorkspaceGateEvidencePath)
            sha256 = $workspaceEvidencePayload.sha256
            status = 'passed'
        }
        database_snapshot = [ordered]@{
            source_filename = $databaseName
            archive_path = "rollback/$databaseName"
            sha256 = $databaseHash
            bytes = $databasePayload.bytes
        }
        source_rollback = [ordered]@{
            schema = $RollbackSource.schema
            scope = 'local_windows_bot_and_operational_tooling'
            source_file_count = $RollbackSource.file_count
            requires_python = $false
            boundary = 'Rust executables and local operational sources are bundled; no Python interpreter or packages are required. Secrets and live state are excluded.'
        }
        bot_disabled = $true
        local_checkpoint_only = $true
        external_output_override = $ExternalOutputOverride
        immutable_release_checkpoint_still_pending_authorization = $true
        payload_files = @($sortedPayload)
        manifest_scope = 'payload files only; control files excluded to avoid a self-hash cycle'
        safety_exclusions = @(
            '.env and credential-like names', 'logs', 'runtime locks', 'heartbeats',
            'operator disabled marker', 'runtime live state'
        )
    }
    $metadataPath = Join-Path $StagingRoot 'ARCHIVE-METADATA.json'
    [IO.File]::WriteAllText(
        $metadataPath, ($metadata | ConvertTo-Json -Depth 30),
        [Text.UTF8Encoding]::new($false, $true)
    )
    $sumLines = @($sortedPayload | ForEach-Object { "$($_.sha256)  $($_.path)" })
    [IO.File]::WriteAllText(
        (Join-Path $StagingRoot 'SHA256SUMS'), (($sumLines -join "`n") + "`n"), $utf8
    )
    $controlFiles = [object[]]@(@('ARCHIVE-METADATA.json', 'SHA256SUMS') | ForEach-Object {
        $path = Join-Path $StagingRoot $_; $item = Get-Item -LiteralPath $path
        [pscustomobject][ordered]@{ path = $_; sha256 = Get-CdrSha256 $path; bytes = [long]$item.Length }
    })
    $null = Publish-CdrCheckpointArchive -StagingRoot $StagingRoot -ArchivePath $ArchivePath
    return [pscustomobject]@{
        RuntimeHash = $runtimeHash
        DatabaseHash = $databaseHash
        DatabaseBytes = $databasePayload.bytes
        OfflineSoakEvidenceHash = $offlineEvidencePayload.sha256
        WorkspaceGateEvidenceHash = $workspaceEvidencePayload.sha256
        SourceFingerprint = $SourceFingerprint.aggregate_sha256
        RollbackSourceFileCount = $RollbackSource.file_count
        PayloadFiles = @($sortedPayload)
        ControlFiles = $controlFiles
        Metadata = $metadata
    }
}

Export-ModuleMember -Function 'New-CdrCheckpointPackage'
