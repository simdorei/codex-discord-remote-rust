# Exercise the real held preflight; only its child-process boundary is replaced.
$script:preflightCalls=0
$script:preflightEvents=[Collections.Generic.List[string]]::new()
$databaseBefore=Get-CdrArtifactHash $State.Compatibility.DatabasePath
$environmentBefore=Get-CdrArtifactHash $EnvPath
$sealBefore=[IO.File]::ReadAllText($seal)
$dispositionBefore=$State.Compatibility.OriginalDisposition

function New-CdrPreflightProof {
    [ordered]@{
        protocol='cdr-recovery-compatibility-v1'
        read_only=$true;compatible=$true;configured_environment_verified=$true
        admission_authorized=$false;recovery_authorized=$false
        new_requests_release_supported=$false;abandonment_apply_supported=$false;special_dispatch_supported=$false
        database=$State.Compatibility.DatabasePath;environment=$EnvPath
    }
}
$script:preflightReply=New-CdrPreflightProof | ConvertTo-Json -Compress
function Invoke-CdrMaintenanceCommand {
    param($s,$File,$Arguments,$LimitSeconds,[switch]$PassThru)
    $expected=@('--admin','check-recovery-compatibility','--repo-root',$RepoRoot,
        '--database',$s.Compatibility.DatabasePath,'--env',$EnvPath)
    if ($File -cne $s.CandidatePath -or $LimitSeconds -ne 30 -or -not $PassThru -or
        ($Arguments -join '|') -cne ($expected -join '|')) { throw 'incorrect readonly child dispatch' }
    $script:preflightCalls++
    $script:preflightEvents.Add('preflight')
    if ($Scenario -eq 'preflight-native') {
        Push-Location -LiteralPath $RepoRoot
        try {
            $lines=& $File @Arguments
            $code=$LASTEXITCODE
        } finally { Pop-Location }
        if ($code -ne 0) { throw "readonly native child failed: $code" }
        # Pass native stdout to production parsing without rewriting any paths.
        $script:preflightReply=$lines -join [Environment]::NewLine
    }
    [pscustomobject]@{Stdout=$script:preflightReply}
}
function Assert-CdrPreflightOutcome([bool]$Allowed,[string]$Label) {
    $beforeCalls=$script:preflightCalls
    $failure=''
    try { Invoke-CdrMaintenancePreflight $State } catch { $failure=$_.Exception.Message }
    if ($script:preflightCalls -ne $beforeCalls+1) { throw "preflight boundary not exercised: $Label" }
    if ($Allowed -and $failure) { throw "$Label rejected: $failure" }
    if (-not $Allowed -and $failure -cne 'maintenance_held_preflight_identity_or_authority_mismatch') {
        throw "$Label did not preserve identity/authority rejection: $failure"
    }
    Write-Output "checked=$Label allowed=$Allowed"
}
function Test-CdrRejectedPreflightPaths {
    foreach ($field in @('database','environment')) {
        $expected=if ($field -ceq 'database') { $State.Compatibility.DatabasePath } else { $EnvPath }
        $leaf=[IO.Path]::GetFileName($expected)
        $otherDrive=if ($expected.StartsWith('C:',[StringComparison]::OrdinalIgnoreCase)) { 'D:' } else { 'C:' }
        $different=@(
            (Join-Path $RepoRoot ('wrong-'+$leaf)),
            (Join-Path (Join-Path $RepoRoot 'other') $leaf),
            ($otherDrive+$expected.Substring(2)))
        $cases=@(
            @{name='null';value=$null},
            @{name='number';value=42},
            @{name='boolean';value=$true},
            @{name='array';value=@($expected)},
            @{name='object';value=@{path=$expected}},
            @{name='empty';value=''},
            @{name='relative';value=$leaf},
            @{name='drive-relative';value=($expected.Substring(0,2)+$leaf)},
            @{name='root-relative';value=('\'+$leaf)},
            @{name='unc';value=('\\server\share\'+$leaf)},
            @{name='verbatim-unc';value=('\\?\UNC\server\share\'+$leaf)},
            @{name='device';value=('\\.\'+$expected)},
            @{name='globalroot';value=('\\?\GLOBALROOT\Device\HarddiskVolume1\'+$leaf)},
            @{name='verbatim-dot';value=('\\?\'+$RepoRoot+'\other\..\'+$leaf)})
        foreach ($path in $different) {
            $cases+=@{name=('different-'+$path);value=$path}
            $cases+=@{name=('different-verbatim-'+$path);value=('\\?\'+$path)}
        }
        foreach ($case in $cases) {
            $proof=New-CdrPreflightProof
            $proof[$field]=$case.value
            $script:preflightReply=$proof | ConvertTo-Json -Depth 8 -Compress
            Assert-CdrPreflightOutcome $false ($field+'-'+$case.name)
        }
        $proof=New-CdrPreflightProof
        $proof.Remove($field)
        $script:preflightReply=$proof | ConvertTo-Json -Compress
        Assert-CdrPreflightOutcome $false ($field+'-missing')
    }
}
function Test-CdrRejectedPreflightAuthority {
    $changes=@{
        protocol='not-the-protocol';read_only=$false;compatible=$false;configured_environment_verified=$false
        admission_authorized=$true;recovery_authorized=$true
        new_requests_release_supported=$true;abandonment_apply_supported=$true;special_dispatch_supported=$true
    }
    foreach ($field in $changes.Keys) {
        $proof=New-CdrPreflightProof
        $proof[$field]=$changes[$field]
        $script:preflightReply=$proof | ConvertTo-Json -Compress
        Assert-CdrPreflightOutcome $false $field
        $proof.Remove($field)
        $script:preflightReply=$proof | ConvertTo-Json -Compress
        Assert-CdrPreflightOutcome $false ($field+'-missing')
    }
}
function Test-CdrRejectedPreflightEngine {
    . (Join-Path $Repository 'scripts/CdrMaintenanceEngine.ps1')
    # Prepared is the last durable phase before drain. Keep the real preflight.
    $State.Phase='prepared'
    $State | Add-Member CompletionPolicy 'runtime-proof-v1'
    $State | Add-Member PreviousCompletedHash ''
    $statePath=Get-CdrMaintenancePath $RepoRoot
    Write-NewCdrMarker $statePath ($State | ConvertTo-Json -Depth 12)
    function Assert-CdrMaintenancePreStopBackup {}
    function Invoke-CdrMaintenanceDrain { $script:preflightEvents.Add('drain') }
    function Invoke-CdrMaintenanceStop { $script:preflightEvents.Add('stop') }
    function Invoke-CdrMaintenancePackaging { $script:preflightEvents.Add('post-stop-backup') }
    function Install-CdrMaintenanceCandidate { $script:preflightEvents.Add('install') }
    function Invoke-CdrMaintenanceLaunch { $script:preflightEvents.Add('launch') }
    function Get-CdrMaintenanceFailureObservation { [pscustomobject]@{fixture=$true} }
    function Publish-CdrMaintenanceFailure { $script:preflightEvents.Add('failure-recorded') }
    $proof=New-CdrPreflightProof
    $proof.database='\\?\'+(Join-Path $RepoRoot 'wrong.sqlite')
    $script:preflightReply=$proof | ConvertTo-Json -Compress
    $guard=Enter-CdrControl -Root $RepoRoot -MaintenanceV2 -Purpose 'maintenance'
    $failure=''
    try {
        try { Invoke-CdrMaintenanceEngine $statePath $operation } catch { $failure=$_.Exception.Message }
        if ($failure -cne 'maintenance_held_preflight_identity_or_authority_mismatch') {
            throw "engine did not reject actual preflight: $failure"
        }
        $persisted=Read-CdrMaintenanceState $statePath
        if (-not $persisted.Halted -or $persisted.Phase -cne 'prepared' -or $persisted.Attempts -ne 1 -or
            $persisted.LastError -cne $failure -or $persisted.Operation -cne $operation -or
            $persisted.Compatibility.OriginalDisposition -cne $dispositionBefore) {
            throw 'preflight engine failure or held ownership was not preserved'
        }
        if (($script:preflightEvents -join ',') -cne 'preflight,failure-recorded') {
            throw ('effect beyond rejected preflight: '+($script:preflightEvents -join ','))
        }
        Write-Output 'engine rejection persisted; drain=0 stop=0 install=0 launch=0'
    } finally { $guard.Dispose() }
}
switch ($Scenario) {
    'preflight-rejected-paths' { Test-CdrRejectedPreflightPaths }
    'preflight-rejected-authority' { Test-CdrRejectedPreflightAuthority }
    'preflight-engine-rejection' { Test-CdrRejectedPreflightEngine }
    'preflight-native' {
        [IO.File]::Copy($BinaryPath,$State.CandidatePath,$false)
        Assert-CdrPreflightOutcome $true $Scenario
        $native=$script:preflightReply | ConvertFrom-Json
        if (-not $native.database.StartsWith('\\?\') -or -not $native.environment.StartsWith('\\?\')) {
            throw 'native Windows canonical paths were not exercised'
        }
    }
    default {
        $proof=New-CdrPreflightProof
        switch ($Scenario) {
            'preflight-normal' {}
            'preflight-database-verbatim' { $proof.database='\\?\'+$proof.database }
            'preflight-environment-verbatim' { $proof.environment='\\?\'+$proof.environment }
            'preflight-both-verbatim' {
                $proof.database='\\?\'+$proof.database
                $proof.environment='\\?\'+$proof.environment
            }
            default { throw 'unknown preflight fixture scenario' }
        }
        $script:preflightReply=$proof | ConvertTo-Json -Compress
        Assert-CdrPreflightOutcome $true $Scenario
    }
}
if ((Get-CdrArtifactHash $State.Compatibility.DatabasePath) -cne $databaseBefore -or
    (Get-CdrArtifactHash $EnvPath) -cne $environmentBefore -or
    [IO.File]::ReadAllText($seal) -cne $sealBefore -or
    $State.Compatibility.OriginalDisposition -cne $dispositionBefore -or
    [IO.File]::Exists($required) -or [IO.File]::Exists($contract) -or
    $script:writers -ne 0 -or $script:starts -ne 0) {
    throw 'preflight changed fixture DB, environment, hold, seal or writer authority'
}
Write-Output "scenario=$Scenario preflight_calls=$script:preflightCalls writers=0 starts=0 DB/env/hold/seal preserved"
