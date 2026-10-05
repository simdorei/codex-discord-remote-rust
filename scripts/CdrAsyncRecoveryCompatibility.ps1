# Candidate-only module. NOT sourced by the operating watchdog.
# The deployment owner must keep its verified maintenance/control lease across
# this check and actual launch. This function never starts the Discord runtime.
Set-StrictMode -Version Latest

function Get-CdrPinnedReadHandle {
    param([Parameter(Mandatory=$true)][string]$Path,
          [Parameter(Mandatory=$true)][string]$ExpectedSha256)
    if ($ExpectedSha256 -cnotmatch '^[a-f0-9]{64}$') { throw 'Invalid pinned artifact SHA-256' }
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Artifact must be a regular non-reparse file'
    }
    $stream = [IO.File]::Open($item.FullName,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try {
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $actual = ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-','').ToLowerInvariant() }
        finally { $sha.Dispose() }
        if ($actual -cne $ExpectedSha256) { throw 'Pinned artifact hash mismatch' }
        $stream.Position = 0
        return $stream
    } catch { $stream.Dispose(); throw }
}

function ConvertTo-CdrComparablePath {
    param([Parameter(Mandatory=$true)][string]$Path)
    if ($Path.StartsWith('\\?\')) { $Path = $Path.Substring(4) }
    return [IO.Path]::GetFullPath($Path)
}

function Assert-CdrAsyncRecoveryCompatibility {
    param(
        [Parameter(Mandatory=$true)][string]$CandidatePath,
        [Parameter(Mandatory=$true)][string]$DatabasePath,
        [Parameter(Mandatory=$true)][string]$CapabilityManifestPath,
        [Parameter(Mandatory=$true)][string]$ExpectedCandidateSha256,
        [Parameter(Mandatory=$true)][string]$ExpectedCapabilitySha256,
        [ValidateRange(1,30)][int]$TimeoutSeconds = 10,
        [string]$EnvironmentPath,
        [string]$ExpectedEnvironmentSha256,
        [string]$WorkingDirectory,
        [switch]$RequireRecoveryPolicy,
        [DateTimeOffset]$DeadlineUtc=[DateTimeOffset]::MaxValue
    )
    $binary = $null; $manifest = $null; $database = $null; $probe = $null; $environment = $null
    try {
        $binary = Get-CdrPinnedReadHandle $CandidatePath $ExpectedCandidateSha256
        $manifest = Get-CdrPinnedReadHandle $CapabilityManifestPath $ExpectedCapabilitySha256
        if ($manifest.Length -gt 16384) { throw 'Capability declaration exceeds bound' }
        $bytes = New-Object byte[] ([int]$manifest.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $read = $manifest.Read($bytes,$offset,$bytes.Length-$offset)
            if ($read -eq 0) { throw 'Incomplete capability declaration' }
            $offset += $read
        }
        $decl = [Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json -ErrorAction Stop
        if ($decl.protocol -cne 'cdr-artifact-capabilities-v1' -or
            $decl.artifact_sha256 -cne $ExpectedCandidateSha256 -or
            -not ($decl.async_resolution_max_format -is [int] -or $decl.async_resolution_max_format -is [long]) -or
            $decl.async_resolution_max_format -ne 1) {
            throw 'Artifact capability is unsupported; candidate was not executed'
        }
        if ($RequireRecoveryPolicy -and (
            -not ($decl.async_recovery_policy_max_format -is [int] -or $decl.async_recovery_policy_max_format -is [long]) -or
            $decl.async_recovery_policy_max_format -ne 1)) {
            throw 'Artifact recovery policy capability is unsupported; candidate was not executed'
        }
        if ($EnvironmentPath) {
            $environment = Get-CdrPinnedReadHandle $EnvironmentPath $ExpectedEnvironmentSha256
            $EnvironmentPath = [IO.Path]::GetFullPath($EnvironmentPath)
            if ($EnvironmentPath.IndexOfAny([char[]]@([char]34,[char]13,[char]10)) -ge 0) {
                throw 'Unsupported environment path characters'
            }
        }
        $dbItem = Get-Item -LiteralPath $DatabasePath -ErrorAction Stop
        if ($dbItem.PSIsContainer -or ($dbItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Database must be a regular non-reparse file'
        }
        # A concurrently open writer prevents this exclusive-read check. Do not
        # kill it or assume a stale PID/maintenance marker establishes quiescence.
        $database = [IO.File]::Open($dbItem.FullName,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
        $candidate = [IO.Path]::GetFullPath($CandidatePath)
        $databasePathFull = $dbItem.FullName
        if ($databasePathFull.IndexOfAny([char[]]@([char]34,[char]13,[char]10)) -ge 0) {
            throw 'Unsupported database path characters'
        }
        $start = New-Object Diagnostics.ProcessStartInfo
        $start.FileName = $candidate
        $start.Arguments = '--admin check-recovery-compatibility --database "' + $databasePathFull + '"'
        if ($EnvironmentPath) { $start.Arguments += ' --env "' + $EnvironmentPath + '"' }
        if ($WorkingDirectory) { $start.WorkingDirectory=[IO.Path]::GetFullPath($WorkingDirectory) }
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        $probe = New-Object Diagnostics.Process
        $probe.StartInfo = $start
        $remaining=[Math]::Min([double]($TimeoutSeconds*1000),($DeadlineUtc-[DateTimeOffset]::UtcNow).TotalMilliseconds)
        if ($remaining -le 0) { throw 'maintenance_deadline_exceeded_before_compatibility_probe' }
        if (-not $probe.Start()) { throw 'Compatibility probe could not start' }
        $stdout = $probe.StandardOutput.ReadToEndAsync()
        $stderr = $probe.StandardError.ReadToEndAsync()
        if (-not $probe.WaitForExit([int][Math]::Floor($remaining))) {
            # Only this newly created, retained Process handle may be stopped.
            $probe.Kill()
            [void]$probe.WaitForExit(3000)
            throw 'Compatibility probe timed out; runtime launch remains held'
        }
        $output = $stdout.GetAwaiter().GetResult()
        $errorText = $stderr.GetAwaiter().GetResult()
        if ($probe.ExitCode -ne 0) { throw ('Compatibility probe rejected candidate: ' + $errorText.Substring(0,[Math]::Min(2048,$errorText.Length))) }
        if ($output.Length -gt 16384) { throw 'Compatibility probe response exceeds bound' }
        $result = $output | ConvertFrom-Json -ErrorAction Stop
        if ($result.protocol -cne 'cdr-recovery-compatibility-v1' -or
            -not ($result.read_only -is [bool]) -or -not $result.read_only -or
            -not ($result.compatible -is [bool]) -or -not $result.compatible -or
            $result.admission_authorized -ne $false -or $result.recovery_authorized -ne $false -or
            $result.supported_async_resolution_format -ne $decl.async_resolution_max_format -or
            $result.required_async_resolution_format -gt $decl.async_resolution_max_format -or
            -not [StringComparer]::OrdinalIgnoreCase.Equals(
                (ConvertTo-CdrComparablePath $result.database),
                (ConvertTo-CdrComparablePath $databasePathFull))) {
            throw 'Compatibility probe identity or capability mismatch'
        }
        if ($RequireRecoveryPolicy -and (
            $result.supported_async_recovery_policy_format -ne $decl.async_recovery_policy_max_format -or
            $result.required_async_recovery_policy_format -gt $decl.async_recovery_policy_max_format)) {
            throw 'Compatibility probe recovery policy capability mismatch'
        }
        if ($EnvironmentPath -and (
            -not ($result.configured_environment_verified -is [bool]) -or
            -not $result.configured_environment_verified -or
            -not [StringComparer]::OrdinalIgnoreCase.Equals(
                (ConvertTo-CdrComparablePath $result.environment),
                (ConvertTo-CdrComparablePath $EnvironmentPath)))) {
            throw 'Compatibility probe did not verify actual startup environment'
        }
        return $result
    } finally {
        if ($null -ne $probe) { $probe.Dispose() }
        if ($null -ne $environment) { $environment.Dispose() }
        if ($null -ne $database) { $database.Dispose() }
        if ($null -ne $manifest) { $manifest.Dispose() }
        if ($null -ne $binary) { $binary.Dispose() }
    }
}
