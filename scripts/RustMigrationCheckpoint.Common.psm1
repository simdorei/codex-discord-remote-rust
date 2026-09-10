$ErrorActionPreference = 'Stop'

function Resolve-CdrCheckpointPath([string]$Base, [string]$Path) {
    if ([IO.Path]::IsPathRooted($Path)) { return [IO.Path]::GetFullPath($Path) }
    return [IO.Path]::GetFullPath((Join-Path $Base $Path))
}

function Get-CdrSha256([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '') }
    finally { $sha.Dispose(); $stream.Dispose() }
}

function Assert-CdrCheckpointLeaf([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label was not found: $Path"
    }
}

function Test-CdrReparsePoint([IO.FileSystemInfo]$Item) {
    return (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)
}

function Assert-CdrPathUnderRoot([string]$Path, [string]$Root, [string]$Label) {
    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $base = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $prefix = $base + [IO.Path]::DirectorySeparatorChar
    if (-not $full.Equals($base, [StringComparison]::OrdinalIgnoreCase) -and
        -not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label must remain under $base`: $full"
    }
    return $full
}

function Assert-CdrSafeDirectoryPath([string]$Path, [string]$Root, [string]$Label) {
    $full = Assert-CdrPathUnderRoot $Path $Root $Label
    $base = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $cursor = $full
    while ($true) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (-not $item.PSIsContainer) { throw "$Label component is not a directory: $cursor" }
            if (Test-CdrReparsePoint $item) {
                throw "$Label component is a reparse point: $cursor"
            }
        }
        if ($cursor.Equals($base, [StringComparison]::OrdinalIgnoreCase)) { break }
        $parent = [IO.Directory]::GetParent($cursor)
        if ($null -eq $parent) { throw "$Label does not reach its required root: $full" }
        $cursor = $parent.FullName.TrimEnd('\', '/')
    }
    return $full
}

function Assert-CdrSafeAbsoluteDirectoryPath([string]$Path, [string]$Label) {
    $full = [IO.Path]::GetFullPath($Path)
    $pathRoot = [IO.Path]::GetPathRoot($full)
    if (-not $full.Equals($pathRoot, [StringComparison]::OrdinalIgnoreCase)) {
        $full = $full.TrimEnd('\', '/')
    }
    $cursor = $full
    while ($true) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (-not $item.PSIsContainer) { throw "$Label component is not a directory: $cursor" }
            if (Test-CdrReparsePoint $item) {
                throw "$Label component is a reparse point: $cursor"
            }
        }
        $parent = [IO.Directory]::GetParent($cursor)
        if ($null -eq $parent) { break }
        $cursor = $parent.FullName
    }
    return $full
}

function Assert-CdrCheckpointSource([string]$Path, [string]$RepoRoot) {
    Assert-CdrCheckpointLeaf $Path 'Checkpoint payload source'
    $full = Assert-CdrPathUnderRoot $Path $RepoRoot 'Checkpoint payload source'
    $base = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\', '/')
    $item = Get-Item -LiteralPath $full -Force
    if (Test-CdrReparsePoint $item) {
        throw "Checkpoint payload source is a reparse point: $full"
    }
    $cursor = $item.Directory
    while ($null -ne $cursor) {
        if (Test-CdrReparsePoint $cursor) {
            throw "Checkpoint payload source has a reparse point ancestor: $($cursor.FullName)"
        }
        $current = [IO.Path]::GetFullPath($cursor.FullName).TrimEnd('\', '/')
        if ($current.Equals($base, [StringComparison]::OrdinalIgnoreCase)) { return }
        $cursor = $cursor.Parent
    }
    throw "Checkpoint payload source does not reach repository root: $full"
}

function Assert-ForbiddenArchiveName([string]$ArchivePath) {
    $normalized = $ArchivePath.Replace('\', '/')
    foreach ($segment in @($normalized.Split('/'))) {
        $lower = $segment.ToLowerInvariant()
        if ($lower -eq '.env' -or $lower.StartsWith('.env.')) {
            throw "Forbidden environment file in checkpoint: $ArchivePath"
        }
        if ($lower -match '(token|secret|password|cookie|credential)') {
            throw "Forbidden credential-like name in checkpoint: $ArchivePath"
        }
        if ($lower.EndsWith('.log')) { throw "Forbidden log in checkpoint: $ArchivePath" }
        if ($lower.EndsWith('.lock') -and $lower -ne 'cargo.lock') {
            throw "Forbidden runtime lock in checkpoint: $ArchivePath"
        }
        if ($lower -match 'heartbeat' -and -not $lower.EndsWith('.ps1')) {
            throw "Forbidden heartbeat state in checkpoint: $ArchivePath"
        }
        if ($lower -in @(
            '.codex_discord_bot.disabled', '.codex_discord_runtime',
            '.codex_discord_runtime.cutover', '.codex_discord_bot.stop',
            '.codex_discord_rust.stop', '.codex_discord_rust.restart'
        )) { throw "Forbidden live runtime state in checkpoint: $ArchivePath" }
    }
}

function Assert-PeMagic([string]$Path) {
    Assert-CdrCheckpointLeaf $Path 'Executable'
    $stream = [IO.File]::OpenRead($Path)
    try {
        if ($stream.Length -lt 2 -or $stream.ReadByte() -ne 0x4D -or $stream.ReadByte() -ne 0x5A) {
            throw "Executable does not have PE MZ magic: $Path"
        }
    } finally { $stream.Dispose() }
}

function Assert-DisabledMarker([string]$Path, [object]$Expected) {
    Assert-CdrCheckpointLeaf $Path 'Operator disabled marker'
    $item = Get-Item -LiteralPath $Path
    $actual = [pscustomobject]@{ Sha256 = Get-CdrSha256 $Path; Bytes = [long]$item.Length }
    if ($null -ne $Expected -and (
        $actual.Sha256 -ne $Expected.Sha256 -or $actual.Bytes -ne $Expected.Bytes
    )) { throw "Operator disabled marker changed during checkpoint creation: $Path" }
    return $actual
}

function Invoke-CdrNativeCapture([string]$FilePath, [string[]]$Arguments) {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $lines = @(& $FilePath @Arguments 2>&1 | ForEach-Object { [string]$_ })
        $exitCode = [int]$LASTEXITCODE
    } finally { $ErrorActionPreference = $previousPreference }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Text = [string](($lines -join "`n").Trim())
        Lines = @($lines)
    }
}

function New-CdrCheckpointTempDirectory([string]$Purpose) {
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $path = Join-Path $tempRoot "cdr-release-checkpoint-$Purpose-$([guid]::NewGuid().ToString('N'))"
    $null = New-Item -ItemType Directory -Path $path
    return [IO.Path]::GetFullPath($path)
}

function Remove-CdrCheckpointTempDirectory([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path -LiteralPath $Path)) { return }
    $resolved = [IO.Path]::GetFullPath($Path)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $leaf = Split-Path -Leaf $resolved
    if (-not $resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        -not $leaf.StartsWith('cdr-release-checkpoint-', [StringComparison]::Ordinal)) {
        throw "Refusing to remove unverified checkpoint temporary directory: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

function Get-CdrForbiddenProcessIdentity([object]$Process, [string]$ExpectedName) {
    $processId = 'unknown'
    try {
        $processId = [string]$Process.Id
        $ticks = $Process.StartTime.ToUniversalTime().Ticks
        return "$ExpectedName|$processId|$ticks"
    } catch {
        throw (
            "Cannot verify forbidden process StartTime: name=$ExpectedName " +
            "pid=$processId error=$($_.Exception.Message)"
        )
    }
}

function Get-CdrForbiddenProcessCandidates {
    [CmdletBinding()]
    param([string]$Name, [scriptblock]$ProcessQuery)
    try {
        if ($null -eq $ProcessQuery) {
            return @(Get-Process -Name $Name -ErrorAction Stop)
        }
        return @(& $ProcessQuery $Name)
    } catch {
        if ($null -eq $ProcessQuery -and
            $_.FullyQualifiedErrorId -like 'NoProcessFoundForGivenName*') { return @() }
        throw "Cannot enumerate forbidden process: name=$Name error=$($_.Exception.Message)"
    }
}

function Get-CdrForbiddenArtifactProcessSnapshot {
    $identities = foreach ($name in @('cdr-runtime', 'cdr-offline-soak', 'cdr-mcp-server')) {
        foreach ($process in @(Get-CdrForbiddenProcessCandidates $name)) {
            try { Get-CdrForbiddenProcessIdentity $process $name }
            finally { $process.Dispose() }
        }
    }
    return @($identities | Sort-Object)
}

function ConvertTo-CdrNativeArgument([string]$Value) {
    $escaped = [regex]::Replace($Value, '(\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\+)$', '$1$1')
    return '"' + $escaped + '"'
}

Export-ModuleMember -Function @(
    'Resolve-CdrCheckpointPath', 'Get-CdrSha256', 'Assert-CdrCheckpointLeaf',
    'Test-CdrReparsePoint', 'Assert-CdrPathUnderRoot', 'Assert-CdrSafeDirectoryPath',
    'Assert-CdrSafeAbsoluteDirectoryPath', 'Assert-CdrCheckpointSource',
    'Assert-ForbiddenArchiveName', 'Assert-PeMagic', 'Assert-DisabledMarker',
    'Invoke-CdrNativeCapture', 'New-CdrCheckpointTempDirectory',
    'Remove-CdrCheckpointTempDirectory', 'Get-CdrForbiddenProcessIdentity',
    'Get-CdrForbiddenProcessCandidates', 'Get-CdrForbiddenArtifactProcessSnapshot',
    'ConvertTo-CdrNativeArgument'
)
