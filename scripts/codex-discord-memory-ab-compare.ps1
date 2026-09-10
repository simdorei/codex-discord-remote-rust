function Read-MemoryAbPhaseSummary {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Role
    )
    $fullPath = Resolve-MemoryAbFullPath -Path $Path
    if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
        throw "$Role summary was not found: $fullPath"
    }
    try {
        $summary = Get-Content -LiteralPath $fullPath -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        throw "$Role summary is not valid JSON: $($_.Exception.Message)"
    }
    if ($summary.schema -cne 'cdr.memory-ab.phase-summary.v1') {
        throw "$Role summary schema mismatch: expected cdr.memory-ab.phase-summary.v1."
    }
    return [pscustomobject]@{ Path = $fullPath; Value = $summary }
}

function Invoke-MemoryAbComparison {
    $baseline = Read-MemoryAbPhaseSummary -Path $BaselineSummaryPath -Role 'baseline'
    $candidate = Read-MemoryAbPhaseSummary -Path $CandidateSummaryPath -Role 'candidate'
    $baselineValue = $baseline.Value
    $candidateValue = $candidate.Value
    Assert-MemoryAbCompletePhaseSummary -Summary $baselineValue -Role 'baseline'
    Assert-MemoryAbCompletePhaseSummary -Summary $candidateValue -Role 'candidate'

    foreach ($key in @('workload_id', 'db_snapshot_id', 'codex_version')) {
        $baselineField = [string]$baselineValue.pairing.$key
        $candidateField = [string]$candidateValue.pairing.$key
        if ([string]::IsNullOrWhiteSpace($baselineField) -or
            [string]::IsNullOrWhiteSpace($candidateField)) {
            throw "$key is required in both comparison summaries."
        }
        if ($baselineField -cne $candidateField) {
            throw "$key mismatch: baseline '$baselineField' candidate '$candidateField'."
        }
    }
    if ([string]$baselineValue.phase -cne [string]$candidateValue.phase) {
        throw (
            "phase mismatch: baseline '$($baselineValue.phase)' " +
            "candidate '$($candidateValue.phase)'."
        )
    }
    foreach ($field in @('requested_duration_seconds', 'sample_interval_seconds')) {
        $baselineSampling = Get-MemoryAbNumericValue `
            -Value $baselineValue.sampling.$field -Location "baseline.sampling.$field"
        $candidateSampling = Get-MemoryAbNumericValue `
            -Value $candidateValue.sampling.$field -Location "candidate.sampling.$field"
        if ($baselineSampling -ne $candidateSampling) {
            throw "$field mismatch between baseline and candidate summaries."
        }
    }

    $baselineKinds = Get-MemoryAbSortedPropertyNames `
        -Value $baselineValue.results -Location 'baseline.results'
    $candidateKinds = Get-MemoryAbSortedPropertyNames `
        -Value $candidateValue.results -Location 'candidate.results'
    Assert-MemoryAbSamePropertyNames -BaselineNames $baselineKinds `
        -CandidateNames $candidateKinds -Location 'results'
    $metricDeltas = [ordered]@{}
    foreach ($kind in $baselineKinds) {
        $baselineMetrics = $baselineValue.results.$kind.metrics
        $candidateMetrics = $candidateValue.results.$kind.metrics
        $baselineMetricNames = Get-MemoryAbSortedPropertyNames `
            -Value $baselineMetrics -Location "baseline.results.$kind.metrics"
        $candidateMetricNames = Get-MemoryAbSortedPropertyNames `
            -Value $candidateMetrics -Location "candidate.results.$kind.metrics"
        Assert-MemoryAbSamePropertyNames -BaselineNames $baselineMetricNames `
            -CandidateNames $candidateMetricNames -Location "results.$kind.metrics"
        $kindDeltas = [ordered]@{}
        foreach ($metric in $baselineMetricNames) {
            $statisticDeltas = [ordered]@{}
            foreach ($statistic in @('mean', 'median', 'max')) {
                $baselineNumber = Get-MemoryAbNumericValue `
                    -Value $baselineMetrics.$metric.$statistic `
                    -Location "baseline.results.$kind.metrics.$metric.$statistic"
                $candidateNumber = Get-MemoryAbNumericValue `
                    -Value $candidateMetrics.$metric.$statistic `
                    -Location "candidate.results.$kind.metrics.$metric.$statistic"
                $statisticDeltas[$statistic] = Get-MemoryAbFiniteDelta `
                    -Candidate $candidateNumber -Baseline $baselineNumber `
                    -Location "results.$kind.metrics.$metric.$statistic"
            }
            $kindDeltas[$metric] = $statisticDeltas
        }
        $metricDeltas[$kind] = $kindDeltas
    }
    $baselineStartup = Get-MemoryAbNumericValue `
        -Value $baselineValue.startup.duration_milliseconds `
        -Location 'baseline.startup.duration_milliseconds'
    $candidateStartup = Get-MemoryAbNumericValue `
        -Value $candidateValue.startup.duration_milliseconds `
        -Location 'candidate.startup.duration_milliseconds'
    if ($baselineStartup -lt 0 -or $candidateStartup -lt 0) {
        throw 'startup duration must be nonnegative in both summaries.'
    }

    $outputPath = Resolve-MemoryAbFullPath -Path $ComparisonOutputPath
    if ($outputPath.Equals($baseline.Path, [StringComparison]::OrdinalIgnoreCase) -or
        $outputPath.Equals($candidate.Path, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'comparison output must not overwrite either input summary.'
    }
    $comparison = [ordered]@{
        schema = 'cdr.memory-ab.comparison.v1'
        created_at_utc = [DateTimeOffset]::UtcNow.ToString('o')
        phase = [string]$baselineValue.phase
        pairing = [ordered]@{
            workload_id = [string]$baselineValue.pairing.workload_id
            db_snapshot_id = [string]$baselineValue.pairing.db_snapshot_id
            codex_version = [string]$baselineValue.pairing.codex_version
        }
        baseline = [ordered]@{
            runtime_label = [string]$baselineValue.runtime_label
            summary_path = $baseline.Path
        }
        candidate = [ordered]@{
            runtime_label = [string]$candidateValue.runtime_label
            summary_path = $candidate.Path
        }
        delta_direction = 'candidate_minus_baseline'
        metric_deltas = $metricDeltas
        startup_deltas = [ordered]@{
            duration_milliseconds = Get-MemoryAbFiniteDelta `
                -Candidate $candidateStartup -Baseline $baselineStartup `
                -Location 'startup.duration_milliseconds'
        }
    }
    Assert-MemoryAbOutputAvailable -Path $outputPath
    Ensure-MemoryAbParentDirectory -Path $outputPath
    Write-MemoryAbNewUtf8File `
        -Path $outputPath -Content ($comparison | ConvertTo-Json -Depth 20)
    Write-Output "comparison_written path=$outputPath"
}
