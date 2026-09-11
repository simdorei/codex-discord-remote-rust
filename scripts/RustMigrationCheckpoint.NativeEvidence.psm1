Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.EvidenceShape.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.SourceBinding.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

function Assert-CdrEvidenceNonemptyString([object]$Value, [string]$Label) {
    Assert-CdrEvidenceCondition ($Value -is [string] -and
        -not [string]::IsNullOrWhiteSpace($Value) -and $Value.Length -le 2048) "$Label must be a nonempty bounded string"
}

function ConvertTo-CdrObservationTime([object]$Value) {
    # PowerShell 7.5+ materializes ISO JSON timestamps as DateTime; 5.1 keeps strings.
    # Only an explicitly UTC typed value is equivalent to the original UTC timestamp.
    if ($Value -is [datetime] -and $Value.Kind -eq [DateTimeKind]::Utc) {
        return [DateTimeOffset]::new($Value)
    }
    $parsed = [DateTimeOffset]::MinValue
    Assert-CdrEvidenceCondition ($Value -is [string] -and
        [DateTimeOffset]::TryParse($Value, [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::None, [ref]$parsed) -and
        $parsed.Offset -eq [TimeSpan]::Zero) 'observation timestamp must be UTC'
    return $parsed
}

function Assert-CdrCurrentPcEvidenceContract([object]$Evidence) {
    $rollbackHash = Get-CdrCheckpointRollbackRecordSha256 $Evidence.rollback_source
    foreach ($part in @('dependency_audit', 'process_observation')) {
        $hash = $Evidence.native_tools.$part.rollback_source_sha256
        Assert-CdrEvidenceHash $hash "$part.rollback_source_sha256"
        Assert-CdrEvidenceCondition ($hash -ceq $rollbackHash) "$part refers to different installation or recovery source"
    }
    $absence = $Evidence.native_tools.python_unavailable_execution
    Assert-CdrEvidenceCondition ($absence.required -is [bool] -and -not $absence.required -and
        (Test-CdrEvidenceExactString $absence.status 'not_run')) 'Python absence test must be optional and explicitly not_run'
    Assert-CdrEvidenceNonemptyString $absence.reason 'Python absence test reason'
    $audit = $Evidence.native_tools.dependency_audit
    Assert-CdrEvidenceCondition ((Test-CdrEvidenceExactString $audit.status 'passed') -and
        (Test-CdrEvidenceExactString $audit.scope 'deliverable_dependencies_and_callsites')) 'dependency audit status or scope is invalid'
    Assert-CdrEvidenceHash $audit.source_fingerprint 'dependency_audit.source_fingerprint'
    Assert-CdrEvidenceCondition ($audit.source_fingerprint -ceq $Evidence.source_fingerprint) 'dependency audit source fingerprint differs'
    Assert-CdrEvidenceNonemptyString $audit.command 'dependency audit command'
    foreach ($name in @('exit_code', 'failed')) {
        Assert-CdrEvidenceCondition ((Test-CdrEvidenceInteger $audit.$name) -and $audit.$name -eq 0) "dependency audit $name must be zero"
    }
    Assert-CdrEvidenceCondition ((Test-CdrEvidenceInteger $audit.passed) -and $audit.passed -gt 0) 'dependency audit must execute positive tests'
    $observation = $Evidence.native_tools.process_observation
    Assert-CdrEvidenceCondition ((Test-CdrEvidenceExactString $observation.schema 'cdr.current-pc-observation.v1') -and
        (Test-CdrEvidenceExactString $observation.status 'completed') -and
        (Test-CdrEvidenceExactString $observation.scope 'owned_descendants_current_pc')) 'process observation schema, status or scope is invalid'
    Assert-CdrEvidenceHash $observation.source_fingerprint 'process_observation.source_fingerprint'
    Assert-CdrEvidenceCondition ($observation.source_fingerprint -ceq $Evidence.source_fingerprint) 'process observation source fingerprint differs'
    $required = @('install_wrappers', 'setup_dry_run', 'pro_helper_offline', 'start_restart_contracts', 'mcp_offline_contracts')
    Assert-CdrEvidenceCondition ($observation.operations -is [array] -and
        $observation.operations.Count -eq $required.Count) 'process observation requires every operation exactly once'
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($operation in $observation.operations) {
        Assert-CdrEvidenceCondition ($operation -is [pscustomobject] -and $operation -isnot [array]) 'operation must be an object'
        Assert-CdrEvidenceCondition ($operation.id -is [string] -and $required -ccontains $operation.id -and
            $seen.Add($operation.id)) 'missing, duplicate or unknown observed operation'
        Assert-CdrEvidenceNonemptyString $operation.command 'operation command'
        Assert-CdrEvidenceCondition ((Test-CdrEvidenceExactString $operation.status 'completed') -and
            (Test-CdrEvidenceExactString $operation.observer 'windows_process_start_stop_trace')) 'operation observation was not completed'
        Assert-CdrEvidenceCondition ((Test-CdrEvidenceInteger $operation.exit_code) -and $operation.exit_code -eq 0) 'observed operation command failed'
        Assert-CdrEvidenceCondition ($operation.canary_observed -is [bool] -and $operation.canary_observed) 'observer canary is missing'
        Assert-CdrEvidenceCondition ((Test-CdrEvidenceInteger $operation.owned_process_starts) -and
            $operation.owned_process_starts -ge 3) 'observation must include both canaries and the actual command'
        $process = $operation.command_process
        Assert-CdrEvidenceCondition ($process -is [pscustomobject] -and $process -isnot [array]) 'command process evidence is missing'
        Assert-CdrEvidenceCondition ($process.observed -is [bool] -and $process.observed -and
            (Test-CdrEvidenceInteger $process.pid) -and $process.pid -gt 0 -and $process.pid -le [uint32]::MaxValue) 'actual command process was not observed'
        Assert-CdrEvidenceNonemptyString $process.name 'command process name'
        Assert-CdrEvidenceCondition ((Test-CdrEvidenceInteger $operation.recognized_python_process_count) -and
            $operation.recognized_python_process_count -eq 0) 'observation contains Python process starts'
        Assert-CdrEvidenceCondition ($operation.live_network_invoked -is [bool] -and
            -not $operation.live_network_invoked) 'live network verification must be recorded separately'
        $start = ConvertTo-CdrObservationTime $operation.started_at
        $end = ConvertTo-CdrObservationTime $operation.ended_at
        $observedStart = ConvertTo-CdrObservationTime $operation.observation_started_at
        $observedEnd = ConvertTo-CdrObservationTime $operation.observation_ended_at
        Assert-CdrEvidenceCondition ($observedStart -le $start -and $start -lt $end -and
            $end -le $observedEnd) 'observation interval does not cover the completed operation'
        $created = ConvertTo-CdrObservationTime $process.created_at
        $exited = ConvertTo-CdrObservationTime $process.exited_at
        $processStart = ConvertTo-CdrObservationTime $process.start_event_at
        $processStop = ConvertTo-CdrObservationTime $process.stop_event_at
        # Notification timestamps can follow native process exit; they must
        # still identify a start/stop pair inside the bounded observation.
        Assert-CdrEvidenceCondition ($start -le $created -and $created -lt $exited -and
            $exited -le $end -and $created -le $processStart -and $processStart -lt $processStop -and
            $processStop -le $observedEnd) 'command process identity or observed lifetime is inconsistent'
    }
    Assert-CdrEvidenceCondition ((Test-CdrEvidenceExactString $Evidence.live_verification.status 'pending') -and
        $Evidence.live_verification.recorded_separately -is [bool] -and
        $Evidence.live_verification.recorded_separately) 'offline gate must leave live verification pending in a separate record'
}

Export-ModuleMember -Function 'Assert-CdrCurrentPcEvidenceContract'
