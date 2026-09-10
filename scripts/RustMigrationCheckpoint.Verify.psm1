Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.RollbackVerify.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.ArchiveContract.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.ProcessSafety.psm1') -ErrorAction Stop
$ErrorActionPreference = 'Stop'

function Test-CdrCheckpointArchive {
    [CmdletBinding()]
    param(
        [string]$ArchivePath,
        [IO.FileStream]$ArchiveStream,
        [string]$VerificationRoot,
        [object[]]$PayloadFiles,
        [object[]]$ControlFiles,
        [Parameter(Mandatory = $true)][object]$ExpectedMetadata,
        [int]$OfflineSmokeDurationSeconds,
        [object[]]$ForbiddenProcessesBefore,
        [string]$DisablePath,
        [object]$MarkerBefore
    )
    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    $extractRoot = Join-Path $VerificationRoot 'extracted'
    $null = New-Item -ItemType Directory -Path $extractRoot
    $archiveContract = Assert-CdrCheckpointArchiveContract `
        -ArchiveStream $ArchiveStream -ExtractRoot $extractRoot `
        -PayloadFiles $PayloadFiles -ControlFiles $ControlFiles `
        -ExpectedMetadata $ExpectedMetadata
    $extracted = Assert-CdrCheckpointExtractedContract `
        -ArchivePath $ArchivePath -ExtractRoot $extractRoot `
        -ArchiveContract $archiveContract -ExpectedMetadata $ExpectedMetadata
    $metadata = $extracted.Metadata
    $metadataPath = $extracted.MetadataPath
    $sumsPath = $extracted.SumsPath

    foreach ($name in @('cdr-runtime.exe', 'cdr-offline-soak.exe', 'cdr-mcp-server.exe')) {
        Assert-PeMagic (Join-Path $extractRoot "artifacts\$name")
    }
    foreach ($required in @(
        'operations\codex-discord-rust-watchdog.ps1',
        'operations\codex-discord-runtime-cutover.ps1',
        'operations\codex-discord-python-runtime.ps1',
        'operations\codex-discord-watchdog.ps1',
        'operations\codex-discord-atomic-file-runtime.ps1',
        'operations\codex-discord-bot-headless.vbs',
        'operations\codex_discord_bot.py', 'operations\install.ps1', 'operations\install.sh',
        'operations\requirements.txt',
        'operations\scripts\New-RustMigrationCheckpoint.ps1',
        'operations\scripts\RustMigrationCheckpoint.Common.psm1',
        'operations\scripts\RustMigrationCheckpoint.Package.psm1',
        'operations\scripts\RustMigrationCheckpoint.Payload.psm1',
        'operations\scripts\RustMigrationCheckpoint.Verify.psm1',
        'operations\scripts\RustMigrationCheckpoint.RollbackVerify.psm1',
        'operations\scripts\RustMigrationCheckpoint.Evidence.psm1',
        'operations\scripts\RustMigrationCheckpoint.EvidenceContract.psm1',
        'operations\scripts\RustMigrationCheckpoint.SourceBinding.psm1',
        'operations\scripts\RustMigrationCheckpoint.Stage.psm1',
        'operations\scripts\RustMigrationCheckpoint.ArchiveContract.psm1',
        'operations\scripts\RustMigrationCheckpoint.ArchivePublish.psm1',
        'operations\scripts\RustMigrationCheckpoint.EvidenceShape.psm1',
        'operations\scripts\RustMigrationCheckpoint.ProcessSafety.psm1',
        'operations\scripts\CodexDiscordSoak.Evidence.psm1',
        'operations\scripts\CodexDiscordSoak.ProcessOwnership.psm1',
        'operations\scripts\CodexDiscordSoak.Provenance.psm1',
        'operations\scripts\CodexDiscordSoak.SourceFingerprint.psm1',
        'operations\scripts\CodexDiscordSoak.WrapperEvidence.psm1',
        'operations\scripts\CodexDiscordSoak.WrapperRuntime.psm1'
    )) { Assert-CdrCheckpointLeaf (Join-Path $extractRoot $required) 'Required checkpoint script' }

    $rollbackPayloadCount = @($metadata.payload_files | Where-Object {
            ([string]$_.path).StartsWith('operations/', [StringComparison]::Ordinal)
        }).Count
    if ($metadata.source_rollback.source_file_count -ne $rollbackPayloadCount) {
        throw 'Checkpoint metadata source rollback count does not match the extracted payload.'
    }
    $rollbackVerification = Test-CdrCheckpointRollbackPayload `
        -ExtractRoot $extractRoot `
        -SourceFileCount $rollbackPayloadCount

    $offlinePath = Join-Path $extractRoot 'artifacts\cdr-offline-soak.exe'
    $smokeRoot = Join-Path $VerificationRoot 'offline-smoke'
    $null = New-Item -ItemType Directory -Path $smokeRoot
    $summaryPath = Join-Path $smokeRoot 'summary.json'
    $eventsPath = Join-Path $smokeRoot 'events.jsonl'
    $arguments = @(
        '--duration-secs', $OfflineSmokeDurationSeconds.ToString(),
        '--seed', '246813579', '--output', $summaryPath, '--events', $eventsPath
    ) | ForEach-Object { ConvertTo-CdrNativeArgument ([string]$_) }
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $offlinePath
    $startInfo.Arguments = $arguments -join ' '
    $startInfo.WorkingDirectory = $smokeRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $sensitiveKeys = @($startInfo.EnvironmentVariables.Keys | Where-Object {
        [string]$_ -match '(?i)(token|secret|password|cookie|api[_-]?key)'
    })
    foreach ($key in $sensitiveKeys) { $startInfo.EnvironmentVariables.Remove([string]$key) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw 'Extracted offline soak executable did not start.' }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(($OfflineSmokeDurationSeconds + 30) * 1000)) {
            $process.Kill()
            $null = $process.WaitForExit(5000)
            throw 'Extracted offline soak executable exceeded its short offline deadline.'
        }
        $process.WaitForExit()
        $null = $stdoutTask.Result
        $stderr = $stderrTask.Result
        if ($process.ExitCode -ne 0) {
            throw "Extracted offline soak failed with exit $($process.ExitCode): $stderr"
        }
    } finally { $process.Dispose() }
    Assert-CdrCheckpointLeaf $summaryPath 'Offline soak summary'
    Assert-CdrCheckpointLeaf $eventsPath 'Offline soak events'
    $summary = [IO.File]::ReadAllText($summaryPath, $utf8) | ConvertFrom-Json
    if ($summary.schema -ne 'cdr.offline-soak.summary.v1' -or
        $summary.mode -ne 'offline_fake_replay' -or $summary.status -ne 'passed') {
        throw 'Extracted offline soak returned an invalid or failing summary.'
    }
    $eventLines = @([IO.File]::ReadAllLines($eventsPath, $utf8) | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_)
    })
    if ($eventLines.Count -eq 0) { throw 'Extracted offline soak emitted no fake/replay events.' }
    Assert-CdrCheckpointBotOff $ForbiddenProcessesBefore 'before_verification'
    $repoRoot = Split-Path -Parent $DisablePath
    $processesAfter = @(Get-CdrCheckpointForbiddenProcessSnapshot -RepoRoot $repoRoot)
    Assert-CdrCheckpointBotOff $processesAfter 'after_verification'
    $markerAfter = Assert-DisabledMarker $DisablePath $MarkerBefore
    $ArchiveStream.Position = 0
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $archiveHash = ([BitConverter]::ToString($sha.ComputeHash($ArchiveStream))).Replace('-', '') }
    finally { $sha.Dispose() }
    return [pscustomobject]@{
        Metadata = $metadata
        MetadataPath = $metadataPath
        SumsPath = $sumsPath
        MarkerAfter = $markerAfter
        SmokeSummary = $summary
        SmokeSummaryPath = $summaryPath
        SmokeEventsPath = $eventsPath
        EventCount = $eventLines.Count
        RollbackVerification = $rollbackVerification
        ArchiveHash = $archiveHash
        ArchiveBytes = [long]$ArchiveStream.Length
    }
}

Export-ModuleMember -Function 'Test-CdrCheckpointArchive'
