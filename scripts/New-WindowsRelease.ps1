[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$')][string]$Version,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\dist'),
    [string]$BinaryDirectory,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$output = [IO.Path]::GetFullPath($OutputDirectory)
Import-Module (Join-Path $PSScriptRoot 'CdrNativeProcess.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'CdrWindowsExecutable.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'CdrWindowsBundle.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'CdrReleasePayload.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'CdrBuildProvenance.psm1') -Force
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
if (-not [IO.Path]::IsPathRooted($target)) { $target = Join-Path $repo $target }
$built = [IO.Path]::GetFullPath((Join-Path $target 'x86_64-pc-windows-msvc\release'))
if (-not $BinaryDirectory) { $BinaryDirectory = $built }
$BinaryDirectory = [IO.Path]::GetFullPath($BinaryDirectory)
if (-not $SkipBuild -and $BinaryDirectory -ine $built) { throw 'Use -SkipBuild for an external binary directory.' }
$name = "codex-discord-remote-$Version-windows-x64"
$archive = Join-Path $output ($name + '.zip')
if ([IO.File]::Exists($archive) -or [IO.File]::Exists($archive + '.sha256')) { throw "Release already exists: $archive" }
$source = Get-CdrBuildSourceState $repo
$commit = $source.Commit
$dirty = $source.State -eq 'dirty'
if ($dirty -and $Version -notmatch '-dev($|\.)') {
    throw 'Publishable releases require a clean source commit. Use a -dev version for local packaging tests.'
}
if (-not $SkipBuild) {
    Push-Location $repo
    try {
        $source = Invoke-CdrRuntimeBuild -Root $repo -BinaryPath (Join-Path $BinaryDirectory 'cdr-runtime.exe') -CargoArguments @('build', '--release', '--locked', '--target',
            'x86_64-pc-windows-msvc', '-p', 'cdr-runtime', '-p', 'cdr-remote-agent', '-p', 'cdr-pro', '--bin', 'cdr-runtime', '--bin', 'cdr-remote-agent', '--bin', 'cdr-pro-helper') `
            -EnvironmentOverrides @{
                CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS='-C target-feature=+crt-static'; VSLANG='1033'
            }
    } finally { Pop-Location }
}
$embedded = Assert-CdrBuildSource -Path (Join-Path $BinaryDirectory 'cdr-runtime.exe') -Source $source
$null = Assert-CdrBuildSource -Path (Join-Path $BinaryDirectory 'cdr-remote-agent.exe') -Source $source
foreach ($binary in @('cdr-runtime.exe', 'cdr-remote-agent.exe', 'cdr-pro-helper.exe')) {
    Test-CdrWindowsExecutable -Path (Join-Path $BinaryDirectory $binary)
}
$null = [IO.Directory]::CreateDirectory($output)
$stage = Join-Path $output ('.release-stage-' + [guid]::NewGuid().ToString('N'))
$payload = Join-Path $stage $name
$null = [IO.Directory]::CreateDirectory($payload)
$utf8 = [Text.UTF8Encoding]::new($false)
try {
    Copy-CdrReleasePayload -RepoRoot $repo -Destination $payload -BinaryDirectory $BinaryDirectory
    $prefix = $payload.TrimEnd('\') + '\'
    $files = @(Get-ChildItem -LiteralPath $payload -Recurse -File -Force | Sort-Object FullName | ForEach-Object {
        [ordered]@{path=$_.FullName.Substring($prefix.Length).Replace('\','/'); sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}
    })
    $manifest = [ordered]@{schema_version=1; version=$Version; platform='windows-x64'; source_commit=$embedded.source_commit; source_tree_dirty=($embedded.source_state -eq 'dirty'); build_info=$embedded; files=$files}
    [IO.File]::WriteAllText((Join-Path $payload 'release-manifest.json'), ($manifest | ConvertTo-Json -Depth 5), $utf8)
    $null = Test-CdrWindowsBundle -Root $payload
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $temporaryZip = Join-Path $stage 'bundle.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($payload, $temporaryZip, [IO.Compression.CompressionLevel]::Optimal, $true)
    [IO.File]::Move($temporaryZip, $archive)
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText(($archive + '.sha256'), "$hash  $name.zip`n", $utf8)
    Write-Output "Windows release created: $archive"
} finally {
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    if (-not $resolvedStage.StartsWith($output.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or
        (Get-Item -LiteralPath $resolvedStage).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Refusing to clean a staging directory outside the release output.'
    }
    Remove-Item -LiteralPath $resolvedStage -Recurse -Force
}
