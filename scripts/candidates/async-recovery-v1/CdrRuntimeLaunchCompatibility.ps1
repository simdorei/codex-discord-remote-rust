# Candidate-only launch integration. Install only with the reviewed watchdog.
# An armed root uses two persistent files. Never remove them for binary recovery.
# Unregistered legacy roots retain their existing startup behavior; final cutover
# must arm the reviewed installation under its real maintenance/control lease.
Set-StrictMode -Version Latest

function Read-CdrPinnedText {
    param([IO.FileStream]$Stream, [int]$Limit)
    if ($Stream.Length -gt $Limit) { throw 'Pinned launch metadata exceeds bound' }
    $Stream.Position=0
    $bytes=New-Object byte[] ([int]$Stream.Length)
    $offset=0
    while ($offset -lt $bytes.Length) {
        $count=$Stream.Read($bytes,$offset,$bytes.Length-$offset)
        if ($count -eq 0) { throw 'Incomplete pinned launch metadata' }
        $offset+=$count
    }
    return [Text.UTF8Encoding]::new($false,$true).GetString($bytes)
}

function Assert-CdrLaunchControl {
    param([string]$Root, $ControlGuard)
    $expected=Join-Path $Root '.codex_discord_rust.control.lock'
    if ($ControlGuard -isnot [IO.FileStream] -or -not $ControlGuard.CanWrite -or
        $ControlGuard.SafeFileHandle.IsClosed -or $ControlGuard.SafeFileHandle.IsInvalid -or
        -not [StringComparer]::OrdinalIgnoreCase.Equals(
            (ConvertTo-CdrComparablePath $ControlGuard.Name),
            (ConvertTo-CdrComparablePath $expected))) {
        throw 'Armed runtime launch requires the retained root control lease'
    }
}

function Invoke-CdrCheckedRuntimeLaunch {
    param(
        [Parameter(Mandatory=$true)][string]$RepoRoot,
        [Parameter(Mandatory=$true)][string]$CandidatePath,
        [Parameter(Mandatory=$true)][string]$EnvironmentPath,
        $ControlGuard,
        [DateTimeOffset]$DeadlineUtc=[DateTimeOffset]::MaxValue,
        [Parameter(Mandatory=$true)][scriptblock]$Launch
    )
    $requiredPath=Join-Path $RepoRoot '.codex_discord_rust.compatibility.required'
    $contractPath=Join-Path $RepoRoot '.codex_discord_rust.compatibility.json'
    $requiredExists=Test-Path -LiteralPath $requiredPath
    $contractExists=Test-Path -LiteralPath $contractPath
    if (-not $requiredExists -and -not $contractExists) { & $Launch; return }
    Assert-CdrLaunchControl -Root $RepoRoot -ControlGuard $ControlGuard
    if (-not $requiredExists -or -not $contractExists) {
        throw 'Incomplete armed compatibility installation; runtime launch held'
    }
    $pins=New-Object 'Collections.Generic.List[IO.FileStream]'
    try {
        $required=Get-CdrPinnedReadHandle $requiredPath ((Get-CdrArtifactHash $requiredPath).ToLowerInvariant())
        $pins.Add($required)
        $expected=(Read-CdrPinnedText $required 128).Trim()
        $contract=Get-CdrPinnedReadHandle $contractPath $expected
        $pins.Add($contract)
        $decl=(Read-CdrPinnedText $contract 16384) | ConvertFrom-Json -ErrorAction Stop
        if ($decl.protocol -cne 'cdr-runtime-launch-v1') { throw 'Unsupported launch contract' }
        foreach ($pair in @(@($decl.root,$RepoRoot),@($decl.candidate_path,$CandidatePath),
                @($decl.environment_path,$EnvironmentPath))) {
            if (-not [IO.Path]::IsPathRooted([string]$pair[0]) -or
                -not [StringComparer]::OrdinalIgnoreCase.Equals(
                    (ConvertTo-CdrComparablePath $pair[0]),(ConvertTo-CdrComparablePath $pair[1]))) {
                throw 'Pinned launch identity differs from actual runtime startup'
            }
        }
        foreach ($path in @($decl.database_path,$decl.capability_path)) {
            if (-not [IO.Path]::IsPathRooted([string]$path)) { throw 'Launch contract paths must be absolute' }
        }
        $pins.Add((Get-CdrPinnedReadHandle $CandidatePath $decl.candidate_sha256))
        $pins.Add((Get-CdrPinnedReadHandle $EnvironmentPath $decl.environment_sha256))
        $pins.Add((Get-CdrPinnedReadHandle $decl.capability_path $decl.capability_sha256))
        $parameters=@{
            CandidatePath=$CandidatePath; DatabasePath=$decl.database_path
            CapabilityManifestPath=$decl.capability_path
            ExpectedCandidateSha256=$decl.candidate_sha256
            ExpectedCapabilitySha256=$decl.capability_sha256
            EnvironmentPath=$EnvironmentPath; ExpectedEnvironmentSha256=$decl.environment_sha256
            WorkingDirectory=$RepoRoot; RequireRecoveryPolicy=$true; DeadlineUtc=$DeadlineUtc
        }
        # The probe releases its DB read handle before the child opens SQLite
        # read/write. Actual control + artifact/config pins remain through Launch.
        $null=Assert-CdrAsyncRecoveryCompatibility @parameters
        Assert-CdrLaunchControl -Root $RepoRoot -ControlGuard $ControlGuard
        if ([DateTimeOffset]::UtcNow -ge $DeadlineUtc) { throw 'maintenance_deadline_exceeded_before_launch' }
        & $Launch
    } finally {
        foreach ($pin in $pins) { $pin.Dispose() }
    }
}
