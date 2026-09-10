Set-StrictMode -Version Latest

if ($null -eq ('CodexDiscordSoak.NativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace CodexDiscordSoak {
    public static class NativeMethods {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true, ExactSpelling = true)]
        public static extern uint GetFinalPathNameByHandleW(
            SafeFileHandle file, StringBuilder path, uint pathLength, uint flags);
    }
}
'@
}

function New-CodexSoakHarnessProvenance {
    param(
        [AllowNull()][string]$RequestedPath,
        [AllowNull()][string]$ExpectedSha256
    )
    $normalizedExpected = if ([string]::IsNullOrWhiteSpace($ExpectedSha256)) {
        $null
    } else {
        $ExpectedSha256.Trim().ToUpperInvariant()
    }
    $record = [ordered]@{
        schema = 'cdr.windows-soak.harness-provenance.v1'
        requested_path = $RequestedPath
        canonical_path = $null
        expected_sha256 = $normalizedExpected
        sha256_before = $null
        sha256_after = $null
        length_before_bytes = $null
        length_after_bytes = $null
        unchanged = $null
        pid = $null
        process_started_at_utc = $null
        process_start_ticks = $null
        actual_process_image_path = $null
        process_exit_confirmed = $null
        verified = $false
        guard_file_access = 'read'
        guard_file_share = 'read'
    }
    [pscustomobject]@{ Record = $record; Guard = $null }
}

function Convert-CodexSoakComparablePath {
    param([AllowNull()][string]$Path)
    if ($null -eq $Path) { return $null }
    $Path = $Path.Replace('/', '\')
    if ($Path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        return '\\' + $Path.Substring(8)
    }
    if ($Path.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) {
        return $Path.Substring(4)
    }
    $Path
}

function Get-CodexSoakFinalPath {
    param([Parameter(Mandatory = $true)][IO.FileStream]$Stream)
    [uint32]$capacity = 512
    while ($true) {
        $buffer = [Text.StringBuilder]::new([int]$capacity)
        $length = [CodexDiscordSoak.NativeMethods]::GetFinalPathNameByHandleW(
            $Stream.SafeFileHandle, $buffer, $capacity, 0)
        if ($length -eq 0) {
            $code = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw [ComponentModel.Win32Exception]::new($code, 'Cannot resolve the opened harness path')
        }
        if ($length -lt $capacity) {
            return Convert-CodexSoakComparablePath $buffer.ToString()
        }
        if ($length -ge 32767) { throw 'Opened harness canonical path is too long' }
        $capacity = $length + 1
    }
}

function Get-CodexSoakGuardSnapshot {
    param([Parameter(Mandatory = $true)][IO.FileStream]$Stream)
    $Stream.Position = 0
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $hash = ([BitConverter]::ToString($sha.ComputeHash($Stream))).Replace('-', '')
        [pscustomobject]@{ Sha256 = $hash; Length = [long]$Stream.Length }
    } finally {
        $sha.Dispose()
        $Stream.Position = 0
    }
}

function Open-CodexSoakHarnessGuard {
    param([Parameter(Mandatory = $true)][object]$Context)
    $Context.Guard = [IO.File]::Open(
        [string]$Context.Record.requested_path,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read)
    $Context.Record.canonical_path = Get-CodexSoakFinalPath $Context.Guard
    $snapshot = Get-CodexSoakGuardSnapshot $Context.Guard
    $Context.Record.sha256_before = $snapshot.Sha256
    $Context.Record.length_before_bytes = $snapshot.Length
    $Context
}

function Throw-CodexSoakProvenanceError {
    param([string]$Code, [string]$Message)
    $exception = [InvalidOperationException]::new($Message)
    $exception.Data['CodexSoakFailureCode'] = $Code
    throw $exception
}

function Assert-CodexSoakCanonicalHarnessPath {
    param(
        [Parameter(Mandatory = $true)][object]$Context,
        [Parameter(Mandatory = $true)][string]$AllowedPath
    )
    if (-not ([string]$Context.Record.canonical_path).Equals(
        (Convert-CodexSoakComparablePath $AllowedPath),
        [StringComparison]::OrdinalIgnoreCase)) {
        Throw-CodexSoakProvenanceError 'canonical_path_mismatch' 'Opened harness canonical path does not match the allowed artifact path'
    }
}

function Assert-CodexSoakExpectedHarnessHash {
    param([Parameter(Mandatory = $true)][object]$Context)
    $expected = $Context.Record.expected_sha256
    if ($null -eq $expected) {
        Throw-CodexSoakProvenanceError 'expected_hash_missing' 'ExpectedHarnessSha256 is required for offline soak evidence'
    }
    if ($expected -notmatch '^[0-9A-F]{64}$') {
        Throw-CodexSoakProvenanceError 'expected_hash_malformed' 'ExpectedHarnessSha256 must contain exactly 64 hexadecimal characters'
    }
    if ($expected -cne $Context.Record.sha256_before) {
        Throw-CodexSoakProvenanceError 'expected_hash_mismatch' 'ExpectedHarnessSha256 does not match the opened offline soak harness'
    }
}

function Set-CodexSoakHarnessProcessIdentity {
    param(
        [Parameter(Mandatory = $true)][object]$Context,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process
    )
    $Process.Refresh()
    $startedAt = $Process.StartTime.ToUniversalTime()
    $Context.Record.pid = [long]$Process.Id
    $Context.Record.process_started_at_utc = $startedAt.ToString('o')
    $Context.Record.process_start_ticks = [long]$startedAt.Ticks
    $actualPath = [string]$Process.Path
    $Context.Record.actual_process_image_path = $actualPath
    $matches = (Convert-CodexSoakComparablePath $actualPath).Equals(
        [string]$Context.Record.canonical_path,
        [StringComparison]::OrdinalIgnoreCase)
    [pscustomobject]@{
        Pid = [int]$Process.Id
        StartedAtUtc = $startedAt
        StartTicks = [long]$startedAt.Ticks
        PathMatches = $matches
        ExpectedPath = [string]$Context.Record.canonical_path
        ActualPath = $actualPath
    }
}

function Assert-CodexSoakHarnessProcessPath {
    param([Parameter(Mandatory = $true)][object]$Identity)
    if (-not $Identity.PathMatches) {
        Throw-CodexSoakProvenanceError 'process_image_mismatch' "Started harness image path mismatch: expected $($Identity.ExpectedPath); actual $($Identity.ActualPath)"
    }
}

function Complete-CodexSoakHarnessProvenance {
    param([Parameter(Mandatory = $true)][object]$Context)
    $snapshot = Get-CodexSoakGuardSnapshot $Context.Guard
    $Context.Record.sha256_after = $snapshot.Sha256
    $Context.Record.length_after_bytes = $snapshot.Length
    $pathMatches = $null -ne $Context.Record.actual_process_image_path -and
        (Convert-CodexSoakComparablePath ([string]$Context.Record.actual_process_image_path)).Equals(
            [string]$Context.Record.canonical_path,
            [StringComparison]::OrdinalIgnoreCase)
    $unchanged = $Context.Record.sha256_before -ceq $Context.Record.sha256_after -and
        $Context.Record.length_before_bytes -eq $Context.Record.length_after_bytes
    $verified = $Context.Record.expected_sha256 -ceq $Context.Record.sha256_before -and
        $unchanged -and $pathMatches
    $Context.Record.unchanged = [bool]$unchanged
    $Context.Record.verified = [bool]$verified
    $Context
}

function Close-CodexSoakHarnessGuard {
    param([Parameter(Mandatory = $true)][object]$Context)
    if ($null -ne $Context.Guard) {
        $Context.Guard.Dispose()
        $Context.Guard = $null
    }
}

Export-ModuleMember -Function @(
    'New-CodexSoakHarnessProvenance',
    'Open-CodexSoakHarnessGuard',
    'Assert-CodexSoakCanonicalHarnessPath',
    'Assert-CodexSoakExpectedHarnessHash',
    'Set-CodexSoakHarnessProcessIdentity',
    'Assert-CodexSoakHarnessProcessPath',
    'Complete-CodexSoakHarnessProvenance',
    'Close-CodexSoakHarnessGuard'
)
