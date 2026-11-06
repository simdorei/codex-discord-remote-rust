function Get-CdrInstallArtifactHash {
    param([string]$Path)
    $stream = [IO.File]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '') }
    finally { $sha.Dispose(); $stream.Dispose() }
}

function Install-CdrSingleBinaryArtifact {
    param([string]$Source, [string]$Destination)
    $expected = Get-CdrInstallArtifactHash $Source
    if ([IO.File]::Exists($Destination)) {
        if ((Get-CdrInstallArtifactHash $Destination) -cne $expected) {
            throw 'Existing runtime differs; use verified deployment to replace it. The installed bot was not stopped or changed.'
        }
        return
    }
    $directory = Split-Path -Parent $Destination
    $null = New-Item -ItemType Directory -Path $directory -Force
    $staged = Join-Path $directory ("cdr-install-" + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::Copy($Source, $staged, $false)
        if ((Get-CdrInstallArtifactHash $staged) -cne $expected) {
            throw 'Runtime artifact changed while staging; installation was not published.'
        }
        # Move refuses an existing destination, including one published by another installer.
        [IO.File]::Move($staged, $Destination)
    } finally {
        if ([IO.File]::Exists($staged)) { [IO.File]::Delete($staged) }
    }
}

function Install-CdrRuntimeArtifact {
    param([string]$RepoRoot, [string]$Source, [switch]$DryRun)
    $ErrorActionPreference = 'Stop'
    $destination = Join-Path $RepoRoot 'target\release\cdr-runtime.exe'
    if ($DryRun) {
        Write-Output 'Would verify the runtime at the launcher path without replacing an installed version.'
        return
    }
    Install-CdrSingleBinaryArtifact -Source $Source -Destination $destination
    $mcpSource = Join-Path (Split-Path -Parent ([IO.Path]::GetFullPath($Source))) 'cdr-remote-agent.exe'
    if ([IO.File]::Exists($mcpSource)) {
        $mcpDestination = Join-Path $RepoRoot 'target\release\cdr-remote-agent.exe'
        $cdrJournal = Join-Path $RepoRoot 'scripts\CdrLaunchJournal.ps1'
        $mcpRunning = $false
        if ([IO.File]::Exists($mcpDestination) -and [IO.File]::Exists($cdrJournal)) {
            try {
                . $cdrJournal
                $mcpObs = Get-CdrComponentObservation -RepoRoot $RepoRoot -Component 'mcp' -BinaryPath $mcpDestination
                $mcpRunning = ($mcpObs.State -eq 'running')
            } catch { }
        }
        if ($mcpRunning) {
            $stagedMcp = Join-Path $RepoRoot '.cdr-maintenance\staged-cdr-remote-agent.exe'
            if ([IO.File]::Exists($stagedMcp)) { [IO.File]::Delete($stagedMcp) }
            Install-CdrSingleBinaryArtifact -Source $mcpSource -Destination $stagedMcp
        } else {
            Install-CdrSingleBinaryArtifact -Source $mcpSource -Destination $mcpDestination
        }
    }
}
Export-ModuleMember -Function Install-CdrRuntimeArtifact, Get-CdrInstallArtifactHash
