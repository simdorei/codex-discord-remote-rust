Set-StrictMode -Version Latest

function New-CodexSoakFailureLedger {
    [pscustomobject]@{
        Primary = $null
        Secondary = [Collections.Generic.List[object]]::new()
    }
}

function New-CodexSoakFailureRecord {
    param([string]$Stage, [string]$Code, [string]$Message)
    [pscustomobject][ordered]@{ stage = $Stage; code = $Code; message = $Message }
}

function Add-CodexSoakFailure {
    param(
        [Parameter(Mandatory)][object]$Ledger,
        [Parameter(Mandatory)][string]$Stage,
        [Parameter(Mandatory)][string]$Code,
        [Parameter(Mandatory)][string]$Message
    )
    $record = New-CodexSoakFailureRecord $Stage $Code $Message
    if ($null -eq $Ledger.Primary) { $Ledger.Primary = $record }
    else { $Ledger.Secondary.Add($record) }
    $record
}

function Get-CodexSoakExceptionCode {
    param([Exception]$Exception, [string]$Stage)
    $value = $Exception.Data['CodexSoakFailureCode']
    if ($value -is [string] -and -not [string]::IsNullOrWhiteSpace($value)) { return $value }
    switch ($Stage) {
        'build' { 'build_failed' }
        'cleanup' { 'cleanup_failed' }
        'write' { 'write_failed' }
        default { "$Stage`_failed" }
    }
}

function Get-CodexSoakOperationalStatus {
    param([AllowNull()][object]$PrimaryFailure)
    if ($null -eq $PrimaryFailure) { return 'passed' }
    if ($PrimaryFailure.stage -in @(
            'source_fingerprint_build_before', 'source_fingerprint_build_after',
            'source_fingerprint_build_compare', 'source_fingerprint_soak_after',
            'source_fingerprint_soak_compare', 'harness_hash_verification')) {
        return 'integrity_failed'
    }
    'runtime_failed'
}

function Sync-CodexSoakFailureState {
    param(
        [Parameter(Mandatory)][object]$Ledger,
        [Parameter(Mandatory)][object]$SourceProvenance
    )
    $SourceProvenance.primary_failure = $Ledger.Primary
    $SourceProvenance.secondary_failures = [object[]]$Ledger.Secondary.ToArray()
    Get-CodexSoakOperationalStatus $Ledger.Primary
}

function New-CodexSoakTerminalException {
    param([Parameter(Mandatory)][object]$Ledger)
    $primary = $Ledger.Primary
    if ($null -eq $primary) { throw 'Cannot create a terminal soak exception without a failure' }
    $failure = [InvalidOperationException]::new([string]$primary.message)
    $failure.Data['CodexSoakFailureStage'] = [string]$primary.stage
    $failure.Data['CodexSoakFailureCode'] = [string]$primary.code
    $failure.Data['CodexSoakPrimaryFailure'] = $primary
    $failure.Data['CodexSoakSecondaryFailures'] = [object[]]$Ledger.Secondary.ToArray()
    $failure
}

function New-CodexSoakSourceProvenance {
    param([AllowNull()][object]$BuildBefore)
    [pscustomobject][ordered]@{
        schema = 'cdr.windows-soak.source-provenance.v1'
        build_before = $BuildBefore
        build_after = $null
        soak_after = $null
        build_fingerprints_equal = $null
        soak_fingerprints_equal = $null
        primary_failure = $null
        secondary_failures = [object[]]@()
    }
}

function Get-CodexSoakEligibilityChecks {
    param(
        [bool]$SameRunBuild, [bool]$CanonicalReleaseHarness,
        [bool]$BuildEqual, [bool]$SoakEqual, [AllowNull()][object]$HarnessProvenance,
        [AllowNull()][object]$ExitCode, [AllowNull()][object]$Harness,
        [int]$EventCount, [long]$DurationSeconds, [double]$RunElapsedSeconds,
        [long]$WarmupSeconds, [double]$SampleIntervalSeconds,
        [double]$MaxSlopeBytesPerHour, [AllowNull()][object]$Regression,
        [bool]$MemoryPassed, [bool]$MarkerPreserved, [bool]$CleanupClean
    )
    $harnessPassed = $null -ne $Harness -and $Harness.schema -eq 'cdr.offline-soak.summary.v1' -and
        $Harness.mode -eq 'offline_fake_replay' -and $Harness.status -eq 'passed'
    $wrapperElapsedReached = -not [double]::IsNaN($RunElapsedSeconds) -and
        -not [double]::IsInfinity($RunElapsedSeconds) -and $RunElapsedSeconds -ge [double]$DurationSeconds
    [pscustomobject][ordered]@{
        same_run_canonical_release_build = $SameRunBuild
        canonical_release_harness = $CanonicalReleaseHarness
        build_fingerprints_equal = $BuildEqual
        soak_fingerprints_equal = $SoakEqual
        harness_provenance_verified = $null -ne $HarnessProvenance -and [bool]$HarnessProvenance.verified
        harness_process_exit_confirmed = $null -ne $HarnessProvenance -and [bool]$HarnessProvenance.process_exit_confirmed
        child_exit_zero = $null -ne $ExitCode -and [long]$ExitCode -eq 0
        harness_contract_passed = $harnessPassed
        event_stream_valid = $EventCount -gt 0
        minimum_duration_met = $DurationSeconds -ge 86400
        harness_duration_matches_request = $harnessPassed -and [long]$Harness.duration_secs -eq $DurationSeconds
        harness_elapsed_reached_request = $harnessPassed -and [decimal]$Harness.elapsed_ms -ge ([decimal]$DurationSeconds * 1000) -and $wrapperElapsedReached
        canonical_warmup = $WarmupSeconds -eq 3600
        sampling_interval_not_weakened = $SampleIntervalSeconds -le 60.0
        slope_limit_not_weakened = $MaxSlopeBytesPerHour -le 1048576.0
        regression_samples_met = $null -ne $Regression -and [int]$Regression.Count -ge 2
        memory_slope_passed = $MemoryPassed
        disabled_marker_preserved = $MarkerPreserved
        cleanup_clean = $CleanupClean
    }
}

function New-CodexSoakSummaryV3 {
    param(
        [string]$OperationalStatus, [string]$EvidenceId, [long]$DurationSeconds,
        [long]$Seed, [AllowNull()][object]$HarnessPid,
        [AllowNull()][object]$HarnessStartedAtUtc, [AllowNull()][object]$ChildExitCode,
        [AllowNull()][object]$HarnessProvenance, [AllowNull()][object]$HarnessSummary,
        [int]$HarnessEventCount, [AllowNull()][object]$Memory,
        [AllowNull()][object]$DisabledMarker, [object]$Outputs,
        [object]$SourceProvenance, [object]$FinalEligibility
    )
    [pscustomobject][ordered]@{
        schema = 'cdr.windows-soak.summary.v3'; mode = 'offline_fake_replay'
        evidence_id = $EvidenceId; operational_status = $OperationalStatus
        duration_seconds = $DurationSeconds; seed = $Seed
        harness_pid = $HarnessPid; harness_started_at_utc = $HarnessStartedAtUtc
        child_exit_code = $ChildExitCode; harness_provenance = $HarnessProvenance
        harness_summary = $HarnessSummary; harness_event_count = $HarnessEventCount
        memory = $Memory; disabled_marker = $DisabledMarker; outputs = $Outputs
        source_provenance = $SourceProvenance; final_eligibility = $FinalEligibility
    }
}

function Publish-CodexSoakSummaryV3 {
    param(
        [Parameter(Mandatory)][object]$Summary,
        [Parameter(Mandatory)][string]$DestinationPath
    )
    try {
        $json = [string]($Summary | ConvertTo-Json -Depth 20)
        $bytes = [Text.UTF8Encoding]::new($false, $true).GetBytes($json)
    } catch {
        $message = "Failed to serialize soak summary for '$DestinationPath': $($_.Exception.Message)"
        throw [InvalidOperationException]::new($message, $_.Exception)
    }

    $destinationFullPath = $DestinationPath
    $stagePath = $null; $stageCreated = $false; $stream = $null
    try {
        $destinationFullPath = [IO.Path]::GetFullPath($DestinationPath)
        $directory = [IO.Path]::GetDirectoryName($destinationFullPath)
        $stageName = ".codex-soak-summary-publish-$([guid]::NewGuid().ToString('n')).tmp"
        $stagePath = Join-Path $directory $stageName
        $stream = [IO.FileStream]::new(
            $stagePath, [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write, [IO.FileShare]::None)
        $stageCreated = $true
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
        $stream.Dispose(); $stream = $null
        [IO.File]::Move($stagePath, $destinationFullPath)
        $stageCreated = $false
    } catch {
        $original = $_.Exception
        $cleanupErrors = [Collections.Generic.List[string]]::new()
        if ($null -ne $stream) {
            try { $stream.Dispose() }
            catch { $cleanupErrors.Add("Staging stream cleanup failed for '$stagePath': $($_.Exception.Message)") }
        }
        if ($stageCreated) {
            try {
                [IO.File]::Delete($stagePath)
            } catch {
                $cleanupErrors.Add("Staging file cleanup failed for '$stagePath': $($_.Exception.Message)")
            }
        }
        $message = "Failed to publish soak summary to '$destinationFullPath': $($original.Message)"
        $failure = [IO.IOException]::new($message, $original)
        $failure.Data['CodexSoakPublicationError'] = $original
        if ($cleanupErrors.Count -gt 0) {
            $failure.Data['CodexSoakCleanupErrors'] = [string[]]$cleanupErrors.ToArray()
        }
        throw $failure
    }
}

function New-CodexSoakSummaryPublicationTerminalException {
    param(
        [Parameter(Mandatory)][object]$Ledger,
        [Parameter(Mandatory)][Exception]$PublicationError
    )
    $null = Add-CodexSoakFailure $Ledger 'write' 'write_failed' $PublicationError.Message
    foreach ($cleanupError in @($PublicationError.Data['CodexSoakCleanupErrors'])) {
        if ($null -ne $cleanupError) {
            $null = Add-CodexSoakFailure $Ledger 'cleanup' 'cleanup_failed' ([string]$cleanupError)
        }
    }
    New-CodexSoakTerminalException $Ledger
}

Export-ModuleMember -Function @(
    'New-CodexSoakFailureLedger', 'Add-CodexSoakFailure',
    'Get-CodexSoakExceptionCode', 'Get-CodexSoakOperationalStatus',
    'Sync-CodexSoakFailureState', 'New-CodexSoakTerminalException',
    'New-CodexSoakSourceProvenance', 'Get-CodexSoakEligibilityChecks',
    'New-CodexSoakSummaryV3', 'Publish-CodexSoakSummaryV3',
    'New-CodexSoakSummaryPublicationTerminalException'
)
