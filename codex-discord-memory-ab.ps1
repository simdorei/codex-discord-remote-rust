[CmdletBinding(DefaultParameterSetName = 'SamplePhase')]
param(
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateSet('warm_idle', 'active', 'recovery')]
    [string]$Phase,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [switch]$AuthorizedLiveMeasurement,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateNotNullOrEmpty()][string]$RuntimeLabel,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateNotNullOrEmpty()][string]$WorkloadId,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateNotNullOrEmpty()][string]$DbSnapshotId,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateNotNullOrEmpty()][string]$CodexVersion,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateRange(1, 2147483647)][int]$BotPid,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [DateTimeOffset]$BotStartedAtUtc,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateNotNullOrEmpty()][string]$BotExecutablePath,
    [Parameter(ParameterSetName = 'SamplePhase')]
    [ValidateRange(1, 2147483647)][int]$AppServerPid,
    [Parameter(ParameterSetName = 'SamplePhase')]
    [DateTimeOffset]$AppServerStartedAtUtc,
    [Parameter(ParameterSetName = 'SamplePhase')]
    [ValidateNotNullOrEmpty()][string]$AppServerExecutablePath,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [DateTimeOffset]$ReadyAtUtc,
    [Parameter(ParameterSetName = 'SamplePhase')]
    [ValidateRange(0.001, 31536000)][double]$DurationSeconds = 300,
    [Parameter(ParameterSetName = 'SamplePhase')]
    [ValidateRange(0.001, 3600)][double]$SampleIntervalSeconds = 1,
    [Parameter(ParameterSetName = 'SamplePhase')]
    [switch]$TestOnlyAllowShortDuration,
    [Parameter(ParameterSetName = 'SamplePhase')]
    [switch]$TestOnlyAllowNonDiscordProcesses,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateNotNullOrEmpty()][string]$RawJsonlPath,
    [Parameter(Mandatory = $true, ParameterSetName = 'SamplePhase')]
    [ValidateNotNullOrEmpty()][string]$SummaryPath,
    [Parameter(Mandatory = $true, ParameterSetName = 'Compare')]
    [ValidateNotNullOrEmpty()][string]$BaselineSummaryPath,
    [Parameter(Mandatory = $true, ParameterSetName = 'Compare')]
    [ValidateNotNullOrEmpty()][string]$CandidateSummaryPath,
    [Parameter(Mandatory = $true, ParameterSetName = 'Compare')]
    [ValidateNotNullOrEmpty()][string]$ComparisonOutputPath
)

$ErrorActionPreference = 'Stop'
$script:MemoryAbUtf8NoBom = [Text.UTF8Encoding]::new($false)
$script:MemoryAbProductionMinimumSeconds = 300.0
$script:MemoryAbInvocationParameters = $PSBoundParameters

$moduleRoot = Join-Path $PSScriptRoot 'scripts'
$modules = @(
    (Join-Path $moduleRoot 'codex-discord-memory-ab-common.ps1'),
    (Join-Path $moduleRoot 'codex-discord-memory-ab-process.ps1'),
    (Join-Path $moduleRoot 'codex-discord-memory-ab-report.ps1'),
    (Join-Path $moduleRoot 'codex-discord-memory-ab-sample.ps1'),
    (Join-Path $moduleRoot 'codex-discord-memory-ab-validation.ps1'),
    (Join-Path $moduleRoot 'codex-discord-memory-ab-compare.ps1')
)
foreach ($module in $modules) {
    if (-not (Test-Path -LiteralPath $module -PathType Leaf)) {
        throw "Memory A/B module was not found: $module"
    }
    . $module
}

if ($PSCmdlet.ParameterSetName -eq 'Compare') {
    Invoke-MemoryAbComparison
} else {
    Invoke-MemoryAbPhaseSampling
}
