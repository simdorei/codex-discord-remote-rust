function Copy-CdrReleasePayload {
    [CmdletBinding()]
    param([string]$RepoRoot, [string]$Destination, [string]$BinaryDirectory)
    $ErrorActionPreference = 'Stop'
    $required = @('install.ps1', 'install.cmd', 'setup-discord-bot.ps1', '.env.example',
        'README.md', 'NOTICE.md', '.agents/plugins/marketplace.json',
        'docs/windows-release.md', 'docs/force-restart.md', 'docs/manual-reserve.md',
        'docs/rust-migration/evidence/drain-certified-artifacts.json')
    $files = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($relative in $required) { $null = $files.Add($relative) }
    # Only tracked, explicit product paths enter a release. Local .env, logs, DBs,
    # build output, credentials and unrelated working-tree files are never scanned.
    $tracked = & git -C $RepoRoot -c core.quotepath=false ls-files -- 'codex-discord-*' 'scripts/*' 'plugins/codex-discord-remote/*'
    if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate release source files' }
    foreach ($relative in $tracked) {
        if ($relative -match '^scripts/tests/' -or $relative -in @('scripts/New-WindowsRelease.ps1',
            'scripts/Test-WindowsRelease.ps1', 'scripts/Publish-WindowsRelease.ps1', 'scripts/CdrReleasePayload.psm1')) { continue }
        if ($relative -match '^codex-discord-.*\.(ps1|cmd|vbs)$' -or
            $relative -match '^scripts/.*\.(ps1|psm1|json)$' -or
            $relative -match '^plugins/codex-discord-remote/') { $null = $files.Add($relative) }
    }
    foreach ($relative in @('scripts/CdrWindowsBundle.psm1', 'scripts/CdrWindowsExecutable.psm1')) { $null = $files.Add($relative) }
    foreach ($relative in ($files | Sort-Object)) {
        $source = Join-Path $RepoRoot $relative
        if (-not [IO.File]::Exists($source)) { throw "Required release source is missing: $relative" }
        if (([IO.File]::GetAttributes($source) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Release source cannot be a linked file: $relative"
        }
        $target = Join-Path $Destination $relative
        $null = [IO.Directory]::CreateDirectory((Split-Path -Parent $target))
        [IO.File]::Copy($source, $target, $false)
    }
    $null = [IO.Directory]::CreateDirectory((Join-Path $Destination 'bin'))
    foreach ($name in @('cdr-runtime.exe', 'cdr-pro-helper.exe')) {
        [IO.File]::Copy((Join-Path $BinaryDirectory $name), (Join-Path $Destination ('bin\' + $name)), $false)
    }
}

Export-ModuleMember -Function Copy-CdrReleasePayload
