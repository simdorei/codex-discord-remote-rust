# The release manifest covers immutable files, never local settings or runtime state.
function Test-CdrWindowsBundle {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Root)
    $ErrorActionPreference = 'Stop'
    $Root = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $manifestPath = Join-Path $Root 'release-manifest.json'
    if (-not [IO.File]::Exists($manifestPath)) {
        throw 'RELEASE_INCOMPLETE: release-manifest.json is missing. Extract the complete Windows release ZIP again.'
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($manifest.schema_version -ne 1 -or $manifest.platform -cne 'windows-x64' -or
        $manifest.source_commit -cnotmatch '^[0-9a-f]{40}$' -or -not $manifest.version -or
        -not $manifest.files -or -not [Environment]::Is64BitOperatingSystem) {
        throw 'RELEASE_INVALID: expected a versioned Windows x64 release manifest on 64-bit Windows.'
    }
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $manifest.files) {
        $relative = [string]$entry.path
        if (-not $relative -or $relative.Contains('\') -or $relative.Contains(':') -or
            $relative.StartsWith('/') -or ($relative.Split('/') | Where-Object { $_ -in @('', '.', '..') }) -or
            $entry.sha256 -cnotmatch '^[A-Fa-f0-9]{64}$' -or -not $seen.Add($relative)) {
            throw "RELEASE_INVALID: unsafe or duplicate manifest entry: $relative"
        }
        $path = [IO.Path]::GetFullPath((Join-Path $Root $relative))
        if (-not $path.StartsWith($Root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "RELEASE_INVALID: file escapes the release directory: $relative"
        }
        if (-not [IO.File]::Exists($path)) { throw "RELEASE_INCOMPLETE: missing file: $relative" }
        $cursor = $path
        while ($cursor -and $cursor -ine $Root) {
            if (([IO.File]::GetAttributes($cursor) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "RELEASE_INVALID: linked file or directory: $relative"
            }
            $cursor = Split-Path -Parent $cursor
        }
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $entry.sha256) {
            throw "RELEASE_DAMAGED: checksum mismatch: $relative. Extract the downloaded ZIP again."
        }
    }
    foreach ($required in @('bin/cdr-runtime.exe', 'bin/cdr-pro-helper.exe', 'install.ps1', 'install.cmd',
        'setup-discord-bot.ps1', '.env.example', '.agents/plugins/marketplace.json',
        'plugins/codex-discord-remote/.codex-plugin/plugin.json', 'scripts/CdrWindowsBundle.psm1')) {
        if (-not $seen.Contains($required)) { throw "RELEASE_INCOMPLETE: manifest omits required file: $required" }
    }
    return $manifest
}

Export-ModuleMember -Function Test-CdrWindowsBundle
