[CmdletBinding()]
param([string]$RepoRoot, [Parameter(Mandatory = $true)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) { $RepoRoot = Join-Path $PSScriptRoot '..' }
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
$OutputPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputPath)
if (Test-Path -LiteralPath $OutputPath) { throw 'Verification output already exists; frozen records are never overwritten.' }
if ([IO.Path]::GetExtension($OutputPath) -cne '.json') { throw 'Verification output must be a JSON record.' }
$rootPrefix = $RepoRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
if ($OutputPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase) -and
    -not $OutputPath.StartsWith((Join-Path $rootPrefix 'docs/rust-migration/evidence/').Replace('/', '\'), [StringComparison]::OrdinalIgnoreCase) -and
    -not $OutputPath.StartsWith((Join-Path $rootPrefix '.release-evidence/').Replace('/', '\'), [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Verification output cannot change bound source; use the ignored evidence directory or an explicit external path.'
}
foreach ($module in @('CdrNativeProcess', 'CdrNativeProcessObservation', 'CodexDiscordSoak.SourceFingerprint',
    'RustMigrationCheckpoint.SourceBinding', 'RustMigrationCheckpoint.NativeEvidence', 'RustMigrationCheckpoint.Quality')) {
    Import-Module (Join-Path $PSScriptRoot "$module.psm1") -ErrorAction Stop
}
$source = Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot
$rollback = Get-CdrCheckpointRollbackSourceRecord -RepoRoot $RepoRoot
$rollbackHash = Get-CdrCheckpointRollbackRecordSha256 $rollback
$processDirectory = [Environment]::CurrentDirectory
Push-Location $RepoRoot
try {
    [Environment]::CurrentDirectory = $RepoRoot
    $auditArgs = @('test', '--locked', '--offline', '-j', '2', '-p', 'cdr-runtime', '--test', 'python_free_repository_contract', '--', '--test-threads=2')
    $auditOutput = Invoke-CdrNative -Executable 'cargo' -Arguments $auditArgs -TimeoutSeconds 600
    $summaries = [regex]::Matches(($auditOutput -join "`n"), 'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;')
    if ($summaries.Count -ne 1 -or $summaries[0].Groups[1].Value -ne '2' -or
        $summaries[0].Groups[2].Value -ne '0' -or $summaries[0].Groups[3].Value -ne '0') {
        throw 'Dependency audit did not execute both required checks without ignores.'
    }
    $common = @('test', '--locked', '--offline', '-j', '2', '-p', 'cdr-runtime')
    $tail = @('--', '--test-threads=2')
    $steps = @(
        @{ id = 'install_wrappers'; exe = 'cargo'; args = $common + @('--test', 'python_free_installer_contract', '--test', 'install_profile_contract', '--test', 'install_profile_location_contract', '--test', 'shell_wrapper_contract') + $tail },
        @{ id = 'setup_dry_run'; exe = 'powershell.exe'; args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $RepoRoot 'setup-discord-bot.ps1'), '-RepoRoot', $RepoRoot, '-DryRun') },
        @{ id = 'pro_helper_offline'; exe = 'cargo'; args = @('test', '--locked', '--offline', '-j', '2', '-p', 'cdr-pro', '--test', 'helper_cli_contract') + $tail },
        @{ id = 'start_restart_contracts'; exe = 'cargo'; args = $common + @('--test', 'startup_contract', '--test', 'python_free_operations_contract', '--test', 'python_free_cutover_contract', '--test', 'restart_boundary_contract') + $tail },
        @{ id = 'mcp_offline_contracts'; exe = 'cargo'; args = @('test', '--locked', '--offline', '-j', '2', '-p', 'cdr-mcp-server', '--test', 'capability_contract', '--test', 'oauth_store_contract') + $tail }
    )
    $operations = [Collections.Generic.List[object]]::new()
    foreach ($step in $steps) {
        Write-Host "Observing native workflow: $($step.id)"
        $result = Invoke-CdrObservedNativeCommand -Id $step.id -Executable $step.exe -Arguments $step.args -OfflineFixture
        if ($step.exe -eq 'cargo') {
            $results = [regex]::Matches($result.stdout, 'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;')
            $expected = if ($step.id -eq 'install_wrappers' -or $step.id -eq 'start_restart_contracts') { 4 } elseif ($step.id -eq 'mcp_offline_contracts') { 2 } else { 1 }
            if ($results.Count -ne $expected) { throw "Missing native test suite result: $($step.id)" }
            foreach ($match in $results) {
                if ([int]$match.Groups[1].Value -le 0 -or $match.Groups[2].Value -ne '0' -or $match.Groups[3].Value -ne '0') {
                    throw "Native workflow contains zero, failed or ignored tests: $($step.id)"
                }
            }
        }
        $result.PSObject.Properties.Remove('stdout')
        $operations.Add($result)
    }
    $after = Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot
    if ($after.aggregate_sha256 -cne $source.aggregate_sha256) { throw 'Rust source changed during verification.' }
    $null = Assert-CdrCheckpointRollbackSourceRecord -Record $rollback -RepoRoot $RepoRoot
    $record = [pscustomobject][ordered]@{
        schema = 'cdr.current-pc-gate-supplement.v1'; kind = 'current_pc_gate_supplement'; status = 'passed'
        source_fingerprint = $source.aggregate_sha256; source_scope = $source | Select-Object schema,file_count,total_bytes
        native_tools = [pscustomobject]@{
            python_unavailable_execution = [pscustomobject]@{ required = $false; status = 'not_run'; reason = 'User approved current-PC verification; shared Python stays installed.' }
            dependency_audit = [pscustomobject]@{ scope = 'deliverable_dependencies_and_callsites'; status = 'passed'; source_fingerprint = $source.aggregate_sha256; rollback_source_sha256 = $rollbackHash; command = 'cargo ' + ($auditArgs -join ' '); exit_code = 0; passed = 2; failed = 0 }
            process_observation = [pscustomobject]@{ schema = 'cdr.current-pc-observation.v1'; status = 'completed'; scope = 'owned_descendants_current_pc'; source_fingerprint = $source.aggregate_sha256; rollback_source_sha256 = $rollbackHash; operations = [object[]]$operations.ToArray() }
        }
        quality = Get-CdrCheckpointQualityRecord -RepoRoot $RepoRoot
        powershell_source = Get-CdrCheckpointPowerShellSourceRecord -RepoRoot $RepoRoot
        rollback_source = $rollback
        live_verification = [pscustomobject]@{ status = 'pending'; recorded_separately = $true }
        limitations = @('Not a full workspace gate or production deployment certificate.', 'Only owned descendants of exercised offline/fixture commands were observed.', 'No claim about renamed interpreters or unexercised paths; live Pro/MCP and Discord checks are separate.')
    }
    Assert-CdrCurrentPcEvidenceContract $record
    if ((Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot).aggregate_sha256 -cne $source.aggregate_sha256) { throw 'Rust source changed before evidence publication.' }
    $null = Assert-CdrCheckpointRollbackSourceRecord -Record $rollback -RepoRoot $RepoRoot
    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    $bytes = $utf8.GetBytes(($record | ConvertTo-Json -Depth 20))
    $stream = [IO.File]::Open($OutputPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
    Write-Output "current_pc_gate_supplement_saved: $OutputPath; not full-workspace or deployment approval"
} finally {
    [Environment]::CurrentDirectory = $processDirectory
    Pop-Location
}
