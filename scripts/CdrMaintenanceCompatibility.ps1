# D4 connects reviewed B6 guards to maintenance without granting incident release.
# Guard scripts set strict mode only in a child scope; legacy maintenance keeps
# its existing optional-state semantics. The actual guard retains strict mode.
function Invoke-CdrProductionCheckedLaunch {
    param([DateTimeOffset]$DeadlineUtc = [DateTimeOffset]::MaxValue,
        [Parameter(Mandatory=$true)][scriptblock]$Launch)
    $childBody=$Launch
    & {
        . (Join-Path $RepoRoot 'scripts/CdrAsyncRecoveryCompatibility.ps1')
        . (Join-Path $RepoRoot 'scripts/CdrRuntimeLaunchCompatibility.ps1')
        Invoke-CdrCheckedRuntimeLaunch -RepoRoot $RepoRoot -CandidatePath $BinaryPath -EnvironmentPath $EnvPath -ControlGuard $controlGuard -DeadlineUtc $DeadlineUtc -Launch {
            # Do not impose a new strict-mode contract on the original child body.
            Set-StrictMode -Off
            & $childBody
        }
    }
}

function Assert-CdrStabilizationArtifacts($State) {
    Assert-CdrStabilizationState $State
    foreach ($pair in @(@('CapabilityPath','CapabilityHash'),@('LaunchManifestPath','LaunchManifestHash'))) {
        if ((Get-CdrArtifactHash $State.Compatibility.($pair[0])) -cne $State.Compatibility.($pair[1])) {
            throw 'maintenance_compatibility_artifact_changed'
        }
    }
}

function Assert-CdrStabilizationArmed($State) {
    Assert-CdrStabilizationArtifacts $State
    foreach ($name in @('.codex_discord_rust.compatibility.required','.codex_discord_rust.compatibility.json')) {
        if (-not [IO.File]::Exists((Join-Path $RepoRoot $name))) {
            throw 'maintenance_compatibility_not_armed; no candidate writer'
        }
    }
}

function Enable-CdrMaintenanceCompatibility($State) {
    Assert-CdrStabilizationArtifacts $State
    if ($State.Phase -cne 'candidate_installed') { throw 'maintenance_compatibility_wrong_phase' }
    Assert-CdrMaintenanceNoRuntime $State
    Assert-CdrMaintenanceArtifacts $State
    Assert-CdrMaintenanceMarkers $State
    Assert-CdrMaintenanceDeadline $State
    & {
        . (Join-Path $RepoRoot 'scripts/CdrAsyncRecoveryCompatibility.ps1')
        . (Join-Path $RepoRoot 'scripts/CdrRuntimeLaunchCompatibility.ps1')
        . (Join-Path $RepoRoot 'scripts/CdrInstallLaunchCompatibility.ps1')
        $c = $State.Compatibility
        $parameters = @{
            RepoRoot=$RepoRoot; CandidatePath=$BinaryPath; EnvironmentPath=$EnvPath
            DatabasePath=$c.DatabasePath; CapabilityManifestPath=$c.CapabilityPath
            ExpectedCandidateSha256=$State.CandidateHash.ToLowerInvariant()
            ExpectedEnvironmentSha256=$State.EnvHash.ToLowerInvariant()
            ExpectedCapabilitySha256=$c.CapabilityHash.ToLowerInvariant()
            ReviewedLaunchManifestPath=$c.LaunchManifestPath
            ExpectedLaunchManifestSha256=$c.LaunchManifestHash.ToLowerInvariant()
            Operation=$State.Operation; ControlGuard=$controlGuard
            DeadlineUtc=[DateTimeOffset]::Parse($State.Deadline)
        }
        $null = Install-CdrRuntimeLaunchCompatibility @parameters
    }
}

# Only canonical local-drive strings and their exact verbatim form are equivalent.
# Never resolve relative paths, UNC/device namespaces or verbatim dot segments.
function Test-CdrStabilizationPath($Observed, $Expected) {
    if ($Observed -isnot [string] -or $Expected -isnot [string]) { return $false }
    $paths=@($Observed,$Expected)
    for ($index=0; $index -lt $paths.Count; $index++) {
        $path=$paths[$index]
        if ($path.StartsWith('\\?\')) {
            if ($path -cnotmatch '\A\\\\\?\\[A-Za-z]:\\') { return $false }
            $path=$path.Substring(4)
        }
        if ($path -cnotmatch '\A[A-Za-z]:\\') { return $false }
        try { $full=[IO.Path]::GetFullPath($path) } catch { return $false }
        if (-not [StringComparer]::OrdinalIgnoreCase.Equals($path,$full)) { return $false }
        $paths[$index]=$full
    }
    return [StringComparer]::OrdinalIgnoreCase.Equals($paths[0],$paths[1])
}

function Invoke-CdrStabilizationProbe($State, [string]$Mode) {
    Assert-CdrStabilizationArtifacts $State
    if ($Mode -ceq 'cleanup') {
        # This deployment does not run any retirement, disposal or release tool.
        Assert-CdrStabilizationArmed $State
        Invoke-CdrProductionCheckedLaunch -DeadlineUtc ([DateTimeOffset]::Parse($State.Deadline)) -Launch {
            Write-Output 'stabilization_cleanup_not_required; original incident remains held'
        }
        return
    }
    if ($Mode -cne 'preflight') { throw 'maintenance_stabilization_probe_mode_invalid' }
    # Live preflight is native/read-only; exclusive DB pinning belongs AFTER
    # normal drain/stop, in Enable-CdrMaintenanceCompatibility.
    $result = Invoke-CdrMaintenanceCommand $State $State.CandidatePath @('--admin','check-recovery-compatibility',
        '--repo-root',$RepoRoot,'--database',$State.Compatibility.DatabasePath,'--env',$EnvPath) 30 -PassThru
    if ($result.Stdout.Length -gt 16384) { throw 'maintenance_preflight_response_too_large' }
    $proof = $result.Stdout | ConvertFrom-Json -ErrorAction Stop
    if ($proof.protocol -cne 'cdr-recovery-compatibility-v1' -or
        $proof.read_only -ne $true -or $proof.compatible -ne $true -or
        $proof.configured_environment_verified -ne $true -or
        $proof.admission_authorized -ne $false -or $proof.recovery_authorized -ne $false -or
        $proof.new_requests_release_supported -ne $false -or
        $proof.abandonment_apply_supported -ne $false -or $proof.special_dispatch_supported -ne $false -or
        -not (Test-CdrStabilizationPath $proof.database $State.Compatibility.DatabasePath) -or
        -not (Test-CdrStabilizationPath $proof.environment $EnvPath)) {
        throw 'maintenance_held_preflight_identity_or_authority_mismatch'
    }
}
