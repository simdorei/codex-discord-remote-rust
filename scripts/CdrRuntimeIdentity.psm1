Import-Module (Join-Path $PSScriptRoot 'CdrBuildProvenance.psm1')
. (Join-Path $PSScriptRoot 'CdrAsyncRecoveryCompatibility.ps1')

function Get-CdrRuntimeArtifact {
    param([string]$Path, [string]$ExpectedBinaryHash, [string]$ExpectedSourceCommit)
    if ($ExpectedBinaryHash -and $ExpectedBinaryHash -notmatch '^[a-fA-F0-9]{64}$') { throw 'Invalid expected binary hash.' }
    if ($ExpectedSourceCommit -and $ExpectedSourceCommit -cnotmatch '^[a-f0-9]{40}$') { throw 'Expected source commit must be a full lowercase SHA.' }
    $hash=Get-CdrBuildFileHash $Path
    if ($ExpectedBinaryHash -and $hash -cne $ExpectedBinaryHash.ToUpperInvariant()) { throw 'Installed runtime differs from the requested artifact; no restart performed.' }
    $pin=Get-CdrPinnedReadHandle $Path $hash.ToLowerInvariant()
    try {
        $info=$null
        try { $info=Read-CdrBuildInfo $Path }
        catch {
            # Old approved binaries have no build-info command. Do not attribute
            # checkout metadata to them, or hide malformed modern metadata.
            if ($_.Exception.Message -notmatch 'unknown argument: --build-info') { throw }
        }
        if ($ExpectedSourceCommit) {
            if ($null -eq $info -or $info.source_commit -cne $ExpectedSourceCommit -or
                $info.source_state -eq 'unknown') { throw 'Embedded source does not match the requested commit.' }
            if ($info.source_state -eq 'dirty' -and -not $ExpectedBinaryHash) {
                throw 'A dirty build requires an explicit binary hash as well as its source commit.'
            }
        }
        return [pscustomobject]@{ ArtifactHash=$hash; BuildInfo=$info; RequireStartupRecord=($null -ne $info) }
    } finally { $pin.Dispose() }
}

function Get-CdrRuntimeArtifactPin {
    param([string]$Path, [string]$ExpectedHash)
    return Get-CdrPinnedReadHandle $Path $ExpectedHash.ToLowerInvariant()
}

function Read-CdrRuntimeStartup {
    param([string]$Root, [string]$BinaryPath, [string]$ExpectedIdentity)
    $path=Join-Path $Root '.codex_discord_rust.runtime.lock'
    if (-not [IO.File]::Exists($path)) { return $null }
    $text=[IO.File]::ReadAllText($path)
    if ($text.Length -gt 16384) { throw 'Runtime startup record exceeds bound.' }
    $lines=@($text -split "`r?`n" | Where-Object { $_.StartsWith('startup=') })
    if ($lines.Count -eq 0) { return $null }
    if ($lines.Count -ne 1) { throw 'Ambiguous runtime startup record.' }
    $record=$lines[0].Substring(8) | ConvertFrom-Json
    if ($record.schema_version -ne 1 -or $record.process_identity -cne $ExpectedIdentity -or
        -not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($record.executable_path),[IO.Path]::GetFullPath($BinaryPath)) -or
        $record.artifact_sha256 -cnotmatch '^[a-f0-9]{64}$') { return $null }
    return $record
}

function Assert-CdrStartedArtifact {
    param([string]$Root, [string]$BinaryPath, [string]$ExpectedIdentity, $Artifact)
    if ($null -eq $Artifact) { return }
    $record=Read-CdrRuntimeStartup $Root $BinaryPath $ExpectedIdentity
    $requires=$Artifact.PSObject.Properties['RequireStartupRecord'] -and $Artifact.RequireStartupRecord
    if ($null -eq $record) {
        if ($requires) { throw 'Replacement has no matching startup artifact record.' }
        return
    }
    if ($record.artifact_sha256 -cne $Artifact.ArtifactHash.ToLowerInvariant()) { throw 'Running child differs from the pinned restart artifact.' }
    if ($Artifact.PSObject.Properties['BuildInfo'] -and $null -ne $Artifact.BuildInfo) {
        foreach($field in @('schema_version','package_version','source_commit','source_tree','source_state')) {
            if ($record.build_info.$field -cne $Artifact.BuildInfo.$field) { throw 'Running child build provenance differs from the requested artifact.' }
        }
    }
}

function Get-CdrProcessIdentity {
    param($Process = $null, [int]$ProcessId = 0)
    $target = $Process
    if ($null -eq $target -and $ProcessId -gt 0) {
        $target = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    }
    if ($null -eq $target) { return $null }
    try {
        $ticks = $target.StartTime.ToUniversalTime().Ticks
        if ($ticks -le 0) { return $null }
        $ticks -= ($ticks % 10)
        return "$($target.Id)|$ticks"
    } catch {
        return $null
    }
}

function Get-CdrProcessObservation {
    param(
        [Parameter(Mandatory)][int]$ProcessId,
        [Parameter(Mandatory)][string]$ExpectedIdentity,
        [Alias('ExpectedPath')][string]$ExpectedBinaryPath = '',
        $Process = $null
    )
    if ($ProcessId -le 0 -or $ExpectedIdentity -notmatch '^(\d+)\|(\d+)$') {
        return [pscustomobject]@{
            State           = 'unknown'
            Matches         = $false
            Process         = $null
            ProcessId       = $ProcessId
            ProcessIdentity = $ExpectedIdentity
            BinaryPath      = $ExpectedBinaryPath
            Reason          = 'invalid_expected_identity'
        }
    }
    if ([int]$Matches[1] -ne $ProcessId) {
        return [pscustomobject]@{
            State           = 'unknown'
            Matches         = $false
            Process         = $null
            ProcessId       = $ProcessId
            ProcessIdentity = $ExpectedIdentity
            BinaryPath      = $ExpectedBinaryPath
            Reason          = 'pid_mismatch'
        }
    }
    $proc = $Process
    if ($null -ne $proc) {
        try {
            if ([int]$proc.Id -ne $ProcessId) {
                $proc = $null
            }
        } catch {
            $proc = $null
        }
    }
    if ($null -eq $proc) {
        try {
            $proc = Get-Process -Id $ProcessId -ErrorAction Stop
        } catch {
            if ($_.FullyQualifiedErrorId -like 'NoProcessFoundForGivenId*' -or
                $_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound -or
                $_.Exception.Message -match 'Cannot find a process with the process identifier') {
                return [pscustomobject]@{
                    State           = 'stopped'
                    Matches         = $false
                    Process         = $null
                    ProcessId       = $ProcessId
                    ProcessIdentity = $ExpectedIdentity
                    BinaryPath      = $ExpectedBinaryPath
                    Reason          = 'not_found'
                }
            }
            return [pscustomobject]@{
                State           = 'unknown'
                Matches         = $false
                Process         = $null
                ProcessId       = $ProcessId
                ProcessIdentity = $ExpectedIdentity
                BinaryPath      = $ExpectedBinaryPath
                Reason          = "get_process_failed:$($_.Exception.GetType().FullName)"
            }
        }
    }
    if ($null -eq $proc) {
        return [pscustomobject]@{
            State           = 'stopped'
            Matches         = $false
            Process         = $null
            ProcessId       = $ProcessId
            ProcessIdentity = $ExpectedIdentity
            BinaryPath      = $ExpectedBinaryPath
            Reason          = 'null_process'
        }
    }
    try {
        try { [void]$proc.Handle } catch { }
        if ($proc.PSObject.Properties['HasExited'] -and $proc.HasExited) {
            return [pscustomobject]@{
                State           = 'stopped'
                Matches         = $false
                Process         = $null
                ProcessId       = $ProcessId
                ProcessIdentity = $ExpectedIdentity
                BinaryPath      = $ExpectedBinaryPath
                Reason          = 'has_exited'
            }
        }
        $actualIdentity = Get-CdrProcessIdentity -Process $proc
        if ([string]::IsNullOrWhiteSpace($actualIdentity)) {
            if ($proc.PSObject.Properties['HasExited'] -and $proc.HasExited) {
                return [pscustomobject]@{
                    State           = 'stopped'
                    Matches         = $false
                    Process         = $null
                    ProcessId       = $ProcessId
                    ProcessIdentity = $ExpectedIdentity
                    BinaryPath      = $ExpectedBinaryPath
                    Reason          = 'exited_during_start_time'
                }
            }
            return [pscustomobject]@{
                State           = 'unknown'
                Matches         = $false
                Process         = $proc
                ProcessId       = $ProcessId
                ProcessIdentity = $ExpectedIdentity
                BinaryPath      = $ExpectedBinaryPath
                Reason          = 'start_time_unavailable'
            }
        }
        if ($actualIdentity -cne $ExpectedIdentity) {
            return [pscustomobject]@{
                State           = 'stopped'
                Matches         = $false
                Process         = $null
                ProcessId       = $ProcessId
                ProcessIdentity = $ExpectedIdentity
                BinaryPath      = $ExpectedBinaryPath
                Reason          = 'identity_reused'
            }
        }
        $resolvedBinary = $ExpectedBinaryPath
        if (-not [string]::IsNullOrWhiteSpace($ExpectedBinaryPath)) {
            $actualPath = $proc.Path
            if ([string]::IsNullOrWhiteSpace($actualPath) -and $proc.PSObject.Properties['MainModule'] -and $null -ne $proc.MainModule) {
                $actualPath = $proc.MainModule.FileName
            }
            if ([string]::IsNullOrWhiteSpace($actualPath)) {
                if ($proc.PSObject.Properties['HasExited'] -and $proc.HasExited) {
                    return [pscustomobject]@{
                        State           = 'stopped'
                        Matches         = $false
                        Process         = $null
                        ProcessId       = $ProcessId
                        ProcessIdentity = $ExpectedIdentity
                        BinaryPath      = $ExpectedBinaryPath
                        Reason          = 'exited_during_path'
                    }
                }
                return [pscustomobject]@{
                    State           = 'unknown'
                    Matches         = $false
                    Process         = $proc
                    ProcessId       = $ProcessId
                    ProcessIdentity = $ExpectedIdentity
                    BinaryPath      = $ExpectedBinaryPath
                    Reason          = 'path_unavailable'
                }
            }
            if (-not [StringComparer]::OrdinalIgnoreCase.Equals(
                [IO.Path]::GetFullPath($actualPath),
                [IO.Path]::GetFullPath($ExpectedBinaryPath)
            )) {
                return [pscustomobject]@{
                    State           = 'unknown'
                    Matches         = $false
                    Process         = $proc
                    ProcessId       = $ProcessId
                    ProcessIdentity = $ExpectedIdentity
                    BinaryPath      = $ExpectedBinaryPath
                    Reason          = 'path_mismatch'
                }
            }
            $resolvedBinary = [IO.Path]::GetFullPath($actualPath)
        }
        return [pscustomobject]@{
            State           = 'running'
            Matches         = $true
            Process         = $proc
            ProcessId       = [int]$ProcessId
            ProcessIdentity = $actualIdentity
            BinaryPath      = $resolvedBinary
            Reason          = 'matched'
        }
    } catch {
        try {
            if ($proc.PSObject.Properties['HasExited'] -and $proc.HasExited) {
                return [pscustomobject]@{
                    State           = 'stopped'
                    Matches         = $false
                    Process         = $null
                    ProcessId       = $ProcessId
                    ProcessIdentity = $ExpectedIdentity
                    BinaryPath      = $ExpectedBinaryPath
                    Reason          = 'exited_exception'
                }
            }
        } catch { }
        return [pscustomobject]@{
            State           = 'unknown'
            Matches         = $false
            Process         = $proc
            ProcessId       = $ProcessId
            ProcessIdentity = $ExpectedIdentity
            BinaryPath      = $ExpectedBinaryPath
            Reason          = "observation_exception:$($_.Exception.GetType().FullName)"
        }
    }
}

function Test-CdrProcessMatchesIdentity {
    param(
        [Parameter(Mandatory)][int]$ProcessId,
        [Parameter(Mandatory)][string]$ExpectedIdentity,
        [Alias('ExpectedPath')][string]$ExpectedBinaryPath = '',
        $Process = $null
    )
    return [bool](Get-CdrProcessObservation -ProcessId $ProcessId -ExpectedIdentity $ExpectedIdentity -ExpectedBinaryPath $ExpectedBinaryPath -Process $Process).Matches
}

function Get-CdrProcessSecurityContext {
    param([Parameter(Mandatory)][int]$ProcessId)
    $proc = Get-Process -Id $ProcessId -ErrorAction Stop
    $sessionId = [int]$proc.SessionId
    $userSid = ''
    if ($ProcessId -eq $PID) {
        $userSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    } else {
        $cimProc = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $ProcessId" -ErrorAction Stop
        if ($null -ne $cimProc) {
            $ownerSidRes = Invoke-CimMethod -InputObject $cimProc -MethodName GetOwnerSid -ErrorAction Stop
            if ($null -ne $ownerSidRes -and $ownerSidRes.ReturnValue -eq 0 -and -not [string]::IsNullOrWhiteSpace($ownerSidRes.Sid)) {
                $userSid = [string]$ownerSidRes.Sid
            }
        }
    }
    return [pscustomobject]@{
        ProcessId = $ProcessId
        SessionId = $sessionId
        UserSid   = $userSid
    }
}

Export-ModuleMember -Function Get-CdrRuntimeArtifact,Get-CdrRuntimeArtifactPin,Read-CdrRuntimeStartup,Assert-CdrStartedArtifact,Get-CdrProcessIdentity,Get-CdrProcessObservation,Test-CdrProcessMatchesIdentity,Get-CdrProcessSecurityContext
