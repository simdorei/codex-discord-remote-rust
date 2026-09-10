[CmdletBinding()]
param(
    [string]$RepoRoot,
    [ValidateRange(1, 604800)][long]$DurationSeconds = 86400,
    [ValidateRange(0.05, 3600.0)][double]$SampleIntervalSeconds = 60.0,
    [ValidateRange(0, 604799)][long]$WarmupSeconds = 3600,
    [ValidateRange(1, 3600)][long]$HarnessExitGraceSeconds = 30,
    [ValidateRange(0, [double]::MaxValue)][double]$MaxSlopeBytesPerHour = 1048576.0,
    [ValidateRange(0, [long]::MaxValue)][long]$Seed = 12345,
    [string]$OutputDirectory, [string]$HarnessPath, [string]$ExpectedHarnessSha256,
    [string]$CargoPath = 'cargo', [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false, $true)
$sourceRoot = [IO.Path]::GetFullPath($PSScriptRoot); if ([string]::IsNullOrWhiteSpace($RepoRoot)) { $RepoRoot = $sourceRoot }
Import-Module (Join-Path $PSScriptRoot 'scripts\CodexDiscordSoak.Provenance.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'scripts\CodexDiscordSoak.ProcessOwnership.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'scripts\CodexDiscordSoak.SourceFingerprint.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'scripts\CodexDiscordSoak.Evidence.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'scripts\CodexDiscordSoak.WrapperRuntime.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'scripts\CodexDiscordSoak.WrapperEvidence.psm1') -Force
$ledger = New-CodexSoakFailureLedger
$sourceProvenance = New-CodexSoakSourceProvenance $null
$child = $null; $childStarted = $false; $childStopped = $false; $processOwner = $null
$memoryWriter = $null; $markerGuard = $null; $provenanceContext = $null; $harnessProvenance = $null; $provenanceCompleted = $false
$stdoutTask = $null; $stderrTask = $null; $harness = $null; $regression = $null
$childPid = $null; $childStartedAt = $null; $childStartTicks = $null; $exitCode = $null
$markerBefore = $null; $markerAfter = $null; $markerPreserved = $false; $cleanupClean = $true
$buildSucceeded = [bool]$SkipBuild; $buildEqual = $null; $soakEqual = $null; $memoryPassed = $false
$eventCount = 0; $runElapsedSeconds = 0.0; $stage = 'prepare'
$evidenceId = [guid]::NewGuid().ToString('n'); $resultPath = $null; $disablePath = $null
$memoryPath = $null; $harnessSummaryPath = $null; $harnessEventsPath = $null
$stdoutPath = $null; $stderrPath = $null; $isRelease = $false
$samples = [Collections.Generic.List[object]]::new(); $memory = $null

try {
    $RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
    if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
        $OutputDirectory = Join-Path $RepoRoot 'target\soak'
    } else { $OutputDirectory = Resolve-CodexSoakRootedPath $RepoRoot $OutputDirectory }
    $null = New-Item -ItemType Directory -Path $OutputDirectory -Force
    $stamp = [datetime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')
    $prefix = Join-Path $OutputDirectory "rust-offline-soak-$stamp-$evidenceId"
    $harnessSummaryPath = "$prefix.harness.summary.json"; $harnessEventsPath = "$prefix.harness.events.jsonl"
    $memoryPath = "$prefix.memory.jsonl"; $resultPath = "$prefix.summary.json"
    $stdoutPath = "$prefix.stdout.log"; $stderrPath = "$prefix.stderr.log"
    $targetRoot = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        Join-Path $sourceRoot 'target'
    } else { Resolve-CodexSoakRootedPath $sourceRoot $env:CARGO_TARGET_DIR }
    if ([string]::IsNullOrWhiteSpace($HarnessPath)) {
        $HarnessPath = Join-Path $targetRoot 'release\cdr-offline-soak.exe'
    } else { $HarnessPath = Resolve-CodexSoakRootedPath $RepoRoot $HarnessPath }
    $releaseHarness = [IO.Path]::GetFullPath((Join-Path $targetRoot 'release\cdr-offline-soak.exe'))
    $debugHarness = [IO.Path]::GetFullPath((Join-Path $targetRoot 'debug\cdr-offline-soak.exe'))
    $isRelease = $HarnessPath.Equals($releaseHarness, [StringComparison]::OrdinalIgnoreCase)
    $isDebugTest = $SkipBuild -and $HarnessPath.Equals($debugHarness, [StringComparison]::OrdinalIgnoreCase)
    if (-not ($isRelease -or $isDebugTest)) { throw "Harness must be the canonical cdr-offline-soak artifact: $HarnessPath" }
    $provenanceContext = New-CodexSoakHarnessProvenance $HarnessPath $ExpectedHarnessSha256
    $harnessProvenance = $provenanceContext.Record
    $disablePath = Join-Path $RepoRoot '.codex_discord_bot.disabled'
    $markerBefore = Get-CodexSoakDisabledMarker $disablePath
    $markerGuard = [IO.File]::Open($disablePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    Assert-CodexSoakRuntimeStopped $RepoRoot

    $stage = 'source_fingerprint_build_before'
    try { $sourceProvenance.build_before = Get-CodexSoakSourceFingerprint -RepoRoot $sourceRoot }
    catch {
        $code = Get-CodexSoakExceptionCode $_.Exception $stage
        $null = Add-CodexSoakFailure $ledger $stage $code $_.Exception.Message
    }
    if (-not $SkipBuild -and $null -eq $ledger.Primary) {
        $stage = 'build'; $buildExit = $null; $buildError = $null
        try {
            Push-Location $sourceRoot
            try { $global:LASTEXITCODE = $null; & $CargoPath build --locked --release -p cdr-runtime --bin cdr-offline-soak
                if ($LASTEXITCODE -is [int]) { $buildExit = [int]$LASTEXITCODE } }
            finally { Pop-Location }
        } catch { $buildError = $_.Exception }
        if ($null -ne $buildError) { $null = Add-CodexSoakFailure $ledger 'build' 'build_failed' $buildError.Message }
        elseif ($null -eq $buildExit) { $null = Add-CodexSoakFailure $ledger 'build' 'build_failed' 'Offline soak harness build failed because no exit code was reported' }
        elseif ($buildExit -eq 0) { $buildSucceeded = $true }
        else { $null = Add-CodexSoakFailure $ledger 'build' 'build_failed' "Offline soak harness build failed with exit $buildExit" }
    }
    $stage = 'source_fingerprint_build_after'
    try { $sourceProvenance.build_after = Get-CodexSoakSourceFingerprint -RepoRoot $sourceRoot }
    catch {
        $code = Get-CodexSoakExceptionCode $_.Exception $stage
        $null = Add-CodexSoakFailure $ledger $stage $code $_.Exception.Message
    }
    if ($null -ne $sourceProvenance.build_before -and $null -ne $sourceProvenance.build_after) {
        $stage = 'source_fingerprint_build_compare'
        try { $buildEqual = [bool](Test-CodexSoakSourceFingerprintEqual $sourceProvenance.build_before $sourceProvenance.build_after) }
        catch {
            $code = Get-CodexSoakExceptionCode $_.Exception $stage
            $null = Add-CodexSoakFailure $ledger $stage $code $_.Exception.Message
        }
        if ($null -ne $buildEqual -and -not $buildEqual) {
            $null = Add-CodexSoakFailure $ledger $stage 'source_changed_during_build' 'Rust source changed during the build boundary'
        }
        if ($null -ne $buildEqual) { $sourceProvenance.build_fingerprints_equal = [bool]$buildEqual }
    }

    if ($null -eq $ledger.Primary) {
        if (-not (Test-Path -LiteralPath $HarnessPath -PathType Leaf)) { throw "Offline soak harness is missing: $HarnessPath" }
        $null = Assert-CodexSoakDisabledMarker $disablePath $markerBefore
        $stage = 'harness_path_verification'
        $null = Open-CodexSoakHarnessGuard $provenanceContext
        Assert-CodexSoakCanonicalHarnessPath $provenanceContext $HarnessPath
        $stage = 'harness_hash_verification'; Assert-CodexSoakExpectedHarnessHash $provenanceContext
        $arguments = @('--duration-secs', $DurationSeconds, '--seed', $Seed,
            '--output', $harnessSummaryPath, '--events', $harnessEventsPath) |
            ForEach-Object { ConvertTo-CodexSoakNativeArgument ([string]$_) }
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $harnessProvenance.canonical_path; $startInfo.Arguments = $arguments -join ' '
        $startInfo.WorkingDirectory = [IO.Path]::GetTempPath(); $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true; $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true; $startInfo.EnvironmentVariables.Remove('DISCORD_BOT_TOKEN')
        $startInfo.EnvironmentVariables.Remove('DISCORD_TOKEN')
        $stage = 'start'; $processOwner = New-CodexSoakProcessOwner; $runClock = [Diagnostics.Stopwatch]::StartNew()
        $child = [Diagnostics.Process]::new(); $child.StartInfo = $startInfo
        if (-not $child.Start()) { throw 'Offline soak harness did not start' }
        $childStarted = $true; $processOwner.Process = $child
        $stdoutTask = $child.StandardOutput.ReadToEndAsync(); $stderrTask = $child.StandardError.ReadToEndAsync()
        $identity = Set-CodexSoakHarnessProcessIdentity $provenanceContext $child
        $childPid = [int]$identity.Pid; $childStartTicks = [long]$identity.StartTicks; $childStartedAt = $identity.StartedAtUtc; $processOwner.Pid = $childPid; $processOwner.StartTicks = $childStartTicks
        Assert-CodexSoakHarnessProcessPath $identity
        $registration = Register-CodexSoakOwnedProcess $processOwner $child
        if ([int]$registration.pid -ne $childPid -or [long]$registration.start_ticks -ne $childStartTicks) { throw 'Owned process registration identity did not match launched harness identity' }
        $sampleClock = [Diagnostics.Stopwatch]::StartNew()
        $effectiveInterval = [math]::Min($SampleIntervalSeconds, [math]::Max(0.05, $DurationSeconds / 10.0))
        $effectiveWarmup = [math]::Min([double]$WarmupSeconds, $DurationSeconds / 4.0)
        $pollMilliseconds = [int][math]::Min(1000, [math]::Ceiling($effectiveInterval * 1000.0))
        $memoryWriter = [IO.StreamWriter]::new($memoryPath, $false, $utf8)
        Add-CodexSoakMemorySample $child $childPid $childStartTicks $childStartedAt $runClock.Elapsed.TotalSeconds $evidenceId $samples $memoryWriter
        $stage = 'run'
        while (-not $child.WaitForExit($pollMilliseconds)) {
            $null = Assert-CodexSoakDisabledMarker $disablePath $markerBefore; Assert-CodexSoakRuntimeStopped $RepoRoot
            if (-not (Test-CodexSoakWithinExitDeadline $runClock.Elapsed $DurationSeconds $HarnessExitGraceSeconds)) { throw 'Offline soak harness exceeded its exit deadline' }
            if ($sampleClock.Elapsed.TotalSeconds -ge $effectiveInterval) {
                Add-CodexSoakMemorySample $child $childPid $childStartTicks $childStartedAt $runClock.Elapsed.TotalSeconds $evidenceId $samples $memoryWriter
                $sampleClock.Restart()
            }
        }
        $child.WaitForExit(); $runClock.Stop(); $runElapsedSeconds = $runClock.Elapsed.TotalSeconds
        if (-not (Test-CodexSoakWithinExitDeadline $runClock.Elapsed $DurationSeconds $HarnessExitGraceSeconds)) { throw 'Offline soak harness exceeded its exit deadline' }
        $exitCode = $child.ExitCode
        if ($exitCode -ne 0) { throw "Offline soak harness failed with exit $exitCode; see $stderrPath" }
        $stage = 'parse'
        if (-not (Test-Path -LiteralPath $harnessSummaryPath -PathType Leaf)) { throw "Harness summary is missing: $harnessSummaryPath" }
        try { $harness = [IO.File]::ReadAllText($harnessSummaryPath, $utf8) | ConvertFrom-Json }
        catch { throw "Invalid harness summary JSON: $($_.Exception.Message)" }
        if ($harness.schema -ne 'cdr.offline-soak.summary.v1' -or $harness.mode -ne 'offline_fake_replay' -or $harness.status -ne 'passed') {
            throw 'Harness summary schema, mode, or status is invalid'
        }
        $eventCount = Get-CodexSoakJsonLineCount $harnessEventsPath $Seed
        $stage = 'evaluate'; $regression = Get-CodexSoakMemoryRegression @($samples) $effectiveWarmup
        $memoryPassed = $regression.SlopeBytesPerHour -le $MaxSlopeBytesPerHour
        $memory = [pscustomobject][ordered]@{
            sample_count = $samples.Count; regression_sample_count = $regression.Count
            sample_interval_seconds = $effectiveInterval; warmup_excluded_seconds = $effectiveWarmup
            working_set_first_bytes = [long]$samples[0].WorkingSetBytes
            working_set_last_bytes = [long]$samples[$samples.Count - 1].WorkingSetBytes
            working_set_max_bytes = [long](($samples | Measure-Object WorkingSetBytes -Maximum).Maximum)
            slope_bytes_per_hour = $regression.SlopeBytesPerHour
            max_slope_bytes_per_hour = $MaxSlopeBytesPerHour; threshold_passed = $memoryPassed
        }
        if (-not $memoryPassed) { throw "Memory slope $($regression.SlopeBytesPerHour) exceeds $MaxSlopeBytesPerHour bytes/hour" }
    }
} catch {
    $failure = $_.Exception
    $code = Get-CodexSoakExceptionCode $failure $stage
    $null = Add-CodexSoakFailure $ledger $stage $code $failure.Message
    foreach ($cleanupError in @($failure.Data['CodexSoakCleanupErrors'])) {
        if ($null -ne $cleanupError) { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' ([string]$cleanupError) }
    }
} finally {
    if ($childStarted) {
        try {
            $termination = Stop-CodexSoakOwnedProcess $processOwner $childPid $childStartTicks 5000
            $childStopped = [bool]$termination.exit_confirmed
            foreach ($cleanupError in @($termination.cleanup_errors)) {
                $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' ([string]$cleanupError)
            }
            if ($childStopped) { $harnessProvenance.process_exit_confirmed = $true; $exitCode = $child.ExitCode }
        } catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message }
    }
    if ($childStopped -and $null -ne $provenanceContext -and $null -ne $provenanceContext.Guard) {
        try {
            $null = Complete-CodexSoakHarnessProvenance $provenanceContext
            $provenanceCompleted = $true
            if (-not $harnessProvenance.verified) {
                $null = Add-CodexSoakFailure $ledger 'harness_hash_verification' 'harness_changed_after_launch' 'Offline soak harness provenance changed after launch'
            }
        } catch {
            $code = Get-CodexSoakExceptionCode $_.Exception 'harness_hash_verification'
            $null = Add-CodexSoakFailure $ledger 'harness_hash_verification' $code $_.Exception.Message
        }
    }
    if ($childStarted -and $childStopped -and $provenanceCompleted) {
        $stage = 'source_fingerprint_soak_after'
        try { $sourceProvenance.soak_after = Get-CodexSoakSourceFingerprint -RepoRoot $sourceRoot }
        catch { $code = Get-CodexSoakExceptionCode $_.Exception $stage; $null = Add-CodexSoakFailure $ledger $stage $code $_.Exception.Message }
        if ($null -ne $sourceProvenance.build_after -and $null -ne $sourceProvenance.soak_after) {
            $stage = 'source_fingerprint_soak_compare'
            try { $soakEqual = [bool](Test-CodexSoakSourceFingerprintEqual $sourceProvenance.build_after $sourceProvenance.soak_after) }
            catch { $code = Get-CodexSoakExceptionCode $_.Exception $stage; $null = Add-CodexSoakFailure $ledger $stage $code $_.Exception.Message }
            if ($null -ne $soakEqual -and -not $soakEqual) {
                $null = Add-CodexSoakFailure $ledger $stage 'source_changed_during_soak' 'Rust source changed during the soak boundary'
            }
            if ($null -ne $soakEqual) { $sourceProvenance.soak_fingerprints_equal = [bool]$soakEqual }
        }
    }
    if ($null -ne $provenanceContext -and (-not $childStarted -or $childStopped)) {
        try { Close-CodexSoakHarnessGuard $provenanceContext }
        catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message }
    }
    if ($null -ne $processOwner -and (-not $childStarted -or $childStopped)) {
        try { Close-CodexSoakProcessOwner $processOwner }
        catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message }
    }
    if ($null -ne $memoryWriter) { try { $memoryWriter.Dispose() } catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message } }
    try { Write-CodexSoakCapturedOutput $stdoutTask $stdoutPath $utf8 } catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message }
    try { Write-CodexSoakCapturedOutput $stderrTask $stderrPath $utf8 } catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message }
    if ($null -ne $child -and (-not $childStarted -or $childStopped)) { try { $child.Dispose() } catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message } }
    if ($null -ne $markerBefore) {
        try { $markerAfter = Assert-CodexSoakDisabledMarker $disablePath $markerBefore; $markerPreserved = $true }
        catch { $cleanupClean = $false; $markerPreserved = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message }
    }
    try { Assert-CodexSoakRuntimeStopped $RepoRoot } catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message }
    if ($null -ne $markerGuard) { try { $markerGuard.Dispose() } catch { $cleanupClean = $false; $null = Add-CodexSoakFailure $ledger 'cleanup' 'cleanup_failed' $_.Exception.Message } }
}

$operationalStatus = Sync-CodexSoakFailureState $ledger $sourceProvenance
$sameRunBuild = -not $SkipBuild -and $isRelease -and $buildSucceeded -and $buildEqual
$checks = $null; $finalEligibility = $null
try {
    $checks = Get-CodexSoakEligibilityChecks $sameRunBuild $isRelease ([bool]$buildEqual) ([bool]$soakEqual) $harnessProvenance $exitCode $harness $eventCount $DurationSeconds $runElapsedSeconds $WarmupSeconds $SampleIntervalSeconds $MaxSlopeBytesPerHour $regression $memoryPassed $markerPreserved $cleanupClean
    $finalEligibility = Get-CodexSoakFinalEligibility -OperationalStatus $operationalStatus -Checks $checks
} catch {
    $null = Add-CodexSoakFailure $ledger 'final_eligibility' 'eligibility_input_invalid' $_.Exception.Message
    $operationalStatus = Sync-CodexSoakFailureState $ledger $sourceProvenance
}
$outputs = [pscustomobject][ordered]@{ summary = $resultPath; memory_jsonl = $memoryPath; harness_summary = $harnessSummaryPath; harness_events = $harnessEventsPath; stdout = $stdoutPath; stderr = $stderrPath }
$disabledMarker = [pscustomobject][ordered]@{ path = $disablePath; sha256_before = $(if ($markerBefore) { $markerBefore.Hash } else { $null }); sha256_after = $(if ($markerAfter) { $markerAfter.Hash } else { $null }); preserved = $markerPreserved }
$startedAtText = if ($childStartedAt) { $childStartedAt.ToString('o') } else { $null }
$finalSummary = New-CodexSoakSummaryV3 $operationalStatus $evidenceId $DurationSeconds $Seed $childPid $startedAtText $exitCode $harnessProvenance $harness $eventCount $memory $disabledMarker $outputs $sourceProvenance $finalEligibility
try { Publish-CodexSoakSummaryV3 $finalSummary $resultPath }
catch { throw (New-CodexSoakSummaryPublicationTerminalException $ledger $_.Exception) }
if ($null -ne $ledger.Primary) { throw (New-CodexSoakTerminalException $ledger) }
Write-Output "soak_passed summary=$resultPath"
