Import-Module (Join-Path $PSScriptRoot 'CdrInstallRuntime.psm1') -ErrorAction Stop

function Install-CdrPluginHelper {
    [CmdletBinding()]
    param([string]$RepoRoot, [string]$RuntimeBinary, [switch]$SkipBuild, [switch]$DryRun)
    $ErrorActionPreference = 'Stop'
    $source = Join-Path (Split-Path -Parent $RuntimeBinary) 'cdr-pro-helper.exe'
    $directory = Join-Path $RepoRoot 'plugins\codex-discord-remote\bin'
    $target = Join-Path $directory 'cdr-pro-helper.exe'
    if ($DryRun) {
        Write-Output 'Would build and stage the Rust Pro helper before installing the plugin.'
        return
    }
    if (-not $SkipBuild) {
        $cargo = Get-Command cargo -ErrorAction Stop
        Push-Location $RepoRoot
        try {
            & $cargo.Source build --release --locked -p cdr-pro --bin cdr-pro-helper
            if ($LASTEXITCODE -ne 0) { throw "Rust Pro helper build failed with exit code $LASTEXITCODE" }
        } finally { Pop-Location }
    }
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "INSTALL_INCOMPLETE: Rust Pro helper was not found: $source"
    }
    $expected = Get-CdrInstallArtifactHash $source
    if ([IO.File]::Exists($target) -and (Get-CdrInstallArtifactHash $target) -ceq $expected) {
        Write-Output 'Staged verified Rust Pro helper.'
        return
    }
    $null = [IO.Directory]::CreateDirectory($directory)
    $staged = Join-Path $directory ('cdr-pro-helper.install.' + [guid]::NewGuid().ToString('N'))
    try {
        [IO.File]::Copy($source, $staged, $false)
        if ((Get-CdrInstallArtifactHash $staged) -cne $expected) {
            throw 'INSTALL_INCOMPLETE: staged Pro helper does not match the built artifact.'
        }
        # Verify before the atomic publication. A failed copy/replace leaves the old helper intact.
        if ([IO.File]::Exists($target)) { [IO.File]::Replace($staged, $target, [NullString]::Value) }
        else { [IO.File]::Move($staged, $target) }
    } finally {
        if ([IO.File]::Exists($staged)) { [IO.File]::Delete($staged) }
    }
    Write-Output 'Staged verified Rust Pro helper.'
}

Export-ModuleMember -Function Install-CdrPluginHelper
