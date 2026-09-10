Set-StrictMode -Version Latest

function Test-CdrEvidenceInteger([AllowNull()][object]$Value) {
    return $Value -is [byte] -or $Value -is [uint16] -or $Value -is [uint32] -or
        $Value -is [uint64] -or $Value -is [sbyte] -or $Value -is [int16] -or
        $Value -is [int32] -or $Value -is [int64]
}

function Test-CdrEvidenceExactString([AllowNull()][object]$Value, [string]$Expected) {
    return $Value -is [string] -and $Value -ceq $Expected
}

function Test-CdrEvidenceRecord([AllowNull()][object]$Value) {
    return $null -ne $Value -and $Value -isnot [array] -and $Value -is [pscustomobject]
}

function Assert-CdrEvidenceCondition([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-CdrEvidenceHash([AllowNull()][object]$Value, [string]$Label) {
    Assert-CdrEvidenceCondition `
        ($Value -is [string] -and $Value -cmatch '^[A-F0-9]{64}$') `
        "$Label must be an uppercase SHA-256 string"
}

function Assert-CdrEvidenceArtifact {
    param([AllowNull()][object]$Artifact, [string]$Label)
    Assert-CdrEvidenceCondition (Test-CdrEvidenceRecord $Artifact) "$Label must be an object"
    Assert-CdrEvidenceHash $Artifact.sha256 "$Label.sha256"
    Assert-CdrEvidenceCondition `
        ((Test-CdrEvidenceInteger $Artifact.bytes) -and [uint64]$Artifact.bytes -gt 0) `
        "$Label.bytes must be a positive integer"
}

function Assert-CdrEvidenceRecordSet([object]$Parent, [string[]]$Names, [string]$Label) {
    foreach ($name in $Names) {
        Assert-CdrEvidenceCondition (Test-CdrEvidenceRecord $Parent.$name) `
            "$Label.$name must be an object"
    }
}

function Assert-CdrOfflineSoakEvidenceShape([AllowNull()][object]$Evidence) {
    Assert-CdrEvidenceCondition (Test-CdrEvidenceRecord $Evidence) 'record must be an object'
    Assert-CdrEvidenceRecordSet $Evidence @(
        'harness', 'memory', 'artifacts', 'provenance', 'safety', 'final_eligibility', 'long_run'
    ) 'record'
    Assert-CdrEvidenceRecordSet $Evidence.artifacts @(
        'cdr_runtime', 'cdr_offline_soak', 'cdr_mcp_server'
    ) 'artifacts'
}

function Assert-CdrWorkspaceGateEvidenceShape([AllowNull()][object]$Evidence) {
    Assert-CdrEvidenceCondition (Test-CdrEvidenceRecord $Evidence) 'record must be an object'
    Assert-CdrEvidenceRecordSet $Evidence @(
        'source_scope', 'rust', 'python', 'quality', 'artifacts', 'safety',
        'powershell_source', 'rollback_source'
    ) 'record'
    Assert-CdrEvidenceRecordSet $Evidence.rust @('release_checkpoint_contracts') 'rust'
    Assert-CdrEvidenceRecordSet $Evidence.python @(
        'pytest_full_suite', 'pro_plugin_contract_suite', 'installer_unittests',
        'desktop_bridge_tests', 'durable_store_tests'
    ) 'python'
    Assert-CdrEvidenceRecordSet $Evidence.artifacts @(
        'cdr_runtime', 'cdr_offline_soak', 'cdr_mcp_server'
    ) 'artifacts'
}

Export-ModuleMember -Function @(
    'Test-CdrEvidenceInteger', 'Test-CdrEvidenceExactString',
    'Assert-CdrEvidenceCondition', 'Assert-CdrEvidenceHash', 'Assert-CdrEvidenceArtifact',
    'Assert-CdrOfflineSoakEvidenceShape', 'Assert-CdrWorkspaceGateEvidenceShape'
)
