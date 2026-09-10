Set-StrictMode -Version Latest

function Throw-CodexSoakSourceFingerprintError {
    param([string]$Code, [string]$Message)
    $exception = [InvalidOperationException]::new($Message)
    $exception.Data['CodexSoakFailureCode'] = $Code
    throw $exception
}

function Assert-CodexSoakSourceNoReparseAncestors {
    param([Parameter(Mandatory = $true)][string]$Path)
    $current = [IO.DirectoryInfo]::new([IO.Path]::GetFullPath($Path))
    while ($null -ne $current) {
        try {
            $attributes = [IO.File]::GetAttributes($current.FullName)
        } catch {
            Throw-CodexSoakSourceFingerprintError 'source_read_failed' "Cannot inspect source path ancestor: $($current.FullName)"
        }
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Throw-CodexSoakSourceFingerprintError 'source_reparse_point' "Source scope contains a reparse point: $($current.FullName)"
        }
        $current = $current.Parent
    }
}

function Assert-CodexSoakSourceContainedPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Candidate
    )
    $full = [IO.Path]::GetFullPath($Candidate)
    $prefix = $RepoRoot
    if (-not $prefix.EndsWith([string][IO.Path]::DirectorySeparatorChar)) {
        $prefix += [IO.Path]::DirectorySeparatorChar
    }
    if (-not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        Throw-CodexSoakSourceFingerprintError 'source_path_escape' "Source path escapes repository root: $full"
    }
}

function Get-CodexSoakSourceLogicalPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$FullPath
    )
    Assert-CodexSoakSourceContainedPath -RepoRoot $RepoRoot -Candidate $FullPath
    $prefixLength = $RepoRoot.Length
    if (-not $RepoRoot.EndsWith([string][IO.Path]::DirectorySeparatorChar)) {
        $prefixLength++
    }
    $FullPath.Substring($prefixLength).Replace('\', '/')
}

function Assert-CodexSoakSourceLogicalPaths {
    param([Parameter(Mandatory = $true)][string[]]$Paths)
    $seen = [Collections.Generic.Dictionary[string, string]]::new(
        [StringComparer]::OrdinalIgnoreCase)
    foreach ($path in $Paths) {
        if ($seen.ContainsKey($path)) {
            if (-not $seen[$path].Equals($path, [StringComparison]::Ordinal)) {
                Throw-CodexSoakSourceFingerprintError 'source_path_collision' "Source paths differ only by case: $($seen[$path]) and $path"
            }
            Throw-CodexSoakSourceFingerprintError 'source_path_duplicate' "Duplicate source path: $path"
        }
        $seen.Add($path, $path)
    }
}

function Add-CodexSoakSourceTree {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$DirectoryPath,
        [Parameter(Mandatory = $true)][Collections.Generic.List[string]]$Files,
        [Parameter(Mandatory = $true)][Collections.Generic.List[string]]$Entries
    )
    Assert-CodexSoakSourceContainedPath -RepoRoot $RepoRoot -Candidate $DirectoryPath
    try {
        $attributes = [IO.File]::GetAttributes($DirectoryPath)
    } catch {
        Throw-CodexSoakSourceFingerprintError 'source_read_failed' "Cannot inspect source directory: $DirectoryPath"
    }
    if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Throw-CodexSoakSourceFingerprintError 'source_reparse_point' "Source scope contains a reparse point: $DirectoryPath"
    }
    if (($attributes -band [IO.FileAttributes]::Directory) -eq 0) {
        Throw-CodexSoakSourceFingerprintError 'source_scope_invalid' "Source scope is not a directory: $DirectoryPath"
    }
    try {
        $children = @([IO.Directory]::EnumerateFileSystemEntries($DirectoryPath))
    } catch {
        Throw-CodexSoakSourceFingerprintError 'source_read_failed' "Cannot enumerate source directory: $DirectoryPath"
    }
    foreach ($child in $children) {
        $full = [IO.Path]::GetFullPath([string]$child)
        Assert-CodexSoakSourceContainedPath -RepoRoot $RepoRoot -Candidate $full
        try {
            $childAttributes = [IO.File]::GetAttributes($full)
        } catch {
            Throw-CodexSoakSourceFingerprintError 'source_read_failed' "Cannot inspect source entry: $full"
        }
        if (($childAttributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Throw-CodexSoakSourceFingerprintError 'source_reparse_point' "Source scope contains a reparse point: $full"
        }
        [void]$Entries.Add((Get-CodexSoakSourceLogicalPath -RepoRoot $RepoRoot -FullPath $full))
        if (($childAttributes -band [IO.FileAttributes]::Directory) -ne 0) {
            Add-CodexSoakSourceTree -RepoRoot $RepoRoot -DirectoryPath $full -Files $Files -Entries $Entries
        } else {
            [void]$Files.Add($full)
        }
    }
}

function Write-CodexSoakU64BigEndian {
    param(
        [Parameter(Mandatory = $true)][IO.Stream]$Stream,
        [Parameter(Mandatory = $true)][uint64]$Value
    )
    [byte[]]$bytes = [BitConverter]::GetBytes($Value)
    if ([BitConverter]::IsLittleEndian) { [Array]::Reverse($bytes) }
    $Stream.Write($bytes, 0, $bytes.Length)
}

function Get-CodexSoakSourceFileRecord {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$LogicalPath
    )
    $stream = $null
    $sha = $null
    try {
        $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        [long]$lengthBefore = $stream.Length
        $sha = [Security.Cryptography.SHA256]::Create()
        [byte[]]$digest = $sha.ComputeHash($stream)
        [long]$lengthAfter = $stream.Length
        if ($lengthBefore -ne $lengthAfter) {
            Throw-CodexSoakSourceFingerprintError 'source_changed_during_snapshot' "Source file changed while hashing: $LogicalPath"
        }
        [pscustomobject]@{
            Path = $LogicalPath
            Bytes = [uint64]$lengthAfter
            Digest = $digest
            Sha256 = ([BitConverter]::ToString($digest)).Replace('-', '')
        }
    } catch {
        if ($_.Exception.Data['CodexSoakFailureCode']) { throw }
        Throw-CodexSoakSourceFingerprintError 'source_read_failed' "Cannot read source file: $LogicalPath"
    } finally {
        if ($null -ne $sha) { $sha.Dispose() }
        if ($null -ne $stream) { $stream.Dispose() }
    }
}

function Get-CodexSoakSourceFingerprint {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    $root = [IO.Path]::GetFullPath($RepoRoot)
    Assert-CodexSoakSourceNoReparseAncestors -Path $root
    $files = [Collections.Generic.List[string]]::new()
    $entries = [Collections.Generic.List[string]]::new()
    foreach ($required in @('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')) {
        $path = [IO.Path]::Combine($root, $required)
        try { $attributes = [IO.File]::GetAttributes($path) } catch [IO.FileNotFoundException] {
            Throw-CodexSoakSourceFingerprintError 'source_required_missing' "Required source file is missing: $required"
        } catch [IO.DirectoryNotFoundException] {
            Throw-CodexSoakSourceFingerprintError 'source_required_missing' "Required source file is missing: $required"
        } catch {
            Throw-CodexSoakSourceFingerprintError 'source_read_failed' "Cannot inspect required source file: $required"
        }
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Throw-CodexSoakSourceFingerprintError 'source_reparse_point' "Required source file is a reparse point: $required"
        }
        if (($attributes -band [IO.FileAttributes]::Directory) -ne 0) {
            Throw-CodexSoakSourceFingerprintError 'source_required_missing' "Required source file is not a regular file: $required"
        }
        [void]$files.Add($path); [void]$entries.Add($required)
    }
    $crates = [IO.Path]::Combine($root, 'crates')
    try { $cratesAttributes = [IO.File]::GetAttributes($crates) } catch [IO.FileNotFoundException] {
        Throw-CodexSoakSourceFingerprintError 'source_scope_missing' 'Required crates source scope is missing'
    } catch [IO.DirectoryNotFoundException] {
        Throw-CodexSoakSourceFingerprintError 'source_scope_missing' 'Required crates source scope is missing'
    } catch {
        Throw-CodexSoakSourceFingerprintError 'source_read_failed' 'Cannot inspect required crates source scope'
    }
    if (($cratesAttributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Throw-CodexSoakSourceFingerprintError 'source_reparse_point' 'Required crates source scope is a reparse point'
    }
    if (($cratesAttributes -band [IO.FileAttributes]::Directory) -eq 0) {
        Throw-CodexSoakSourceFingerprintError 'source_scope_invalid' 'Required crates source scope is not a directory'
    }
    [void]$entries.Add('crates')
    Add-CodexSoakSourceTree -RepoRoot $root -DirectoryPath $crates -Files $files -Entries $entries
    $cargo = [IO.Path]::Combine($root, '.cargo')
    try { $cargoAttributes = [IO.File]::GetAttributes($cargo); $hasCargo = $true } catch [IO.FileNotFoundException] {
        $hasCargo = $false
    } catch [IO.DirectoryNotFoundException] {
        $hasCargo = $false
    } catch {
        Throw-CodexSoakSourceFingerprintError 'source_read_failed' 'Cannot inspect optional .cargo source scope'
    }
    if ($hasCargo) {
        if (($cargoAttributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Throw-CodexSoakSourceFingerprintError 'source_reparse_point' 'Optional .cargo source scope is a reparse point'
        }
        [void]$entries.Add('.cargo')
        Add-CodexSoakSourceTree -RepoRoot $root -DirectoryPath $cargo -Files $files -Entries $entries
    }
    Assert-CodexSoakSourceLogicalPaths -Paths @($entries)
    $paths = [Collections.Generic.List[string]]::new()
    $fullByPath = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::Ordinal)
    foreach ($file in $files) {
        $logical = Get-CodexSoakSourceLogicalPath -RepoRoot $root -FullPath $file
        [void]$paths.Add($logical); $fullByPath.Add($logical, $file)
    }
    $paths.Sort([StringComparer]::Ordinal)
    $strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
    $frame = [IO.MemoryStream]::new()
    $domain = [Text.Encoding]::ASCII.GetBytes("cdr.rust-source-fingerprint.v1`0")
    $frame.Write($domain, 0, $domain.Length)
    Write-CodexSoakU64BigEndian -Stream $frame -Value ([uint64]$paths.Count)
    $records = [Collections.Generic.List[object]]::new(); [uint64]$total = 0
    foreach ($logical in $paths) {
        try { [byte[]]$pathBytes = $strictUtf8.GetBytes($logical) } catch {
            Throw-CodexSoakSourceFingerprintError 'source_path_encoding_failed' "Source path cannot be encoded as strict UTF-8: $logical"
        }
        $record = Get-CodexSoakSourceFileRecord -Path $fullByPath[$logical] -LogicalPath $logical
        Write-CodexSoakU64BigEndian -Stream $frame -Value ([uint64]$pathBytes.Length)
        $frame.Write($pathBytes, 0, $pathBytes.Length)
        Write-CodexSoakU64BigEndian -Stream $frame -Value $record.Bytes
        $frame.Write($record.Digest, 0, $record.Digest.Length)
        [void]$records.Add([pscustomobject][ordered]@{ path = $record.Path; bytes = $record.Bytes; sha256 = $record.Sha256 })
        $total += $record.Bytes
    }
    $aggregate = [Security.Cryptography.SHA256]::Create()
    try { $aggregateHex = ([BitConverter]::ToString($aggregate.ComputeHash($frame.ToArray()))).Replace('-', '') } finally {
        $aggregate.Dispose(); $frame.Dispose()
    }
    [pscustomobject][ordered]@{
        schema = 'cdr.rust-source-fingerprint.v1'; repo_root = $root
        aggregate_sha256 = $aggregateHex; file_count = [uint64]$paths.Count
        total_bytes = $total; files = @($records)
    }
}

Export-ModuleMember -Function 'Get-CodexSoakSourceFingerprint'
